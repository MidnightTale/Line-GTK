//! 保存する認証情報はSecret Serviceへ渡す。設定ファイルやコマンド引数へ書かない。
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};

const LIMIT: usize = 16_384;
#[derive(Serialize, Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}
impl Credentials {
    pub fn valid(&self) -> bool {
        let email = self.email.trim();
        self.email.len() <= 320
            && !email.chars().any(char::is_whitespace)
            && email.split_once('@').is_some_and(|(user, domain)| {
                !user.is_empty()
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
                    && !domain.contains('@')
            })
            && !self.password.is_empty()
            && self.password.len() <= 4096
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    Unavailable,
    Failed,
    Timeout,
    Invalid,
}
pub enum Action {
    Load,
    Save(Credentials),
    Clear,
}
type Reply = async_channel::Sender<Result<Option<Credentials>, StoreError>>;

pub fn request(action: Action) -> async_channel::Receiver<Result<Option<Credentials>, StoreError>> {
    static WORKER: OnceLock<mpsc::Sender<(Action, Reply)>> = OnceLock::new();
    let (tx, rx) = async_channel::bounded(1);
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<(Action, Reply)>();
        std::thread::spawn(move || {
            // 保存と削除をFIFOで処理し、遅い保存が削除後に情報を復活させない。
            for (action, reply) in rx {
                let _ = reply.send_blocking(perform(action));
            }
        });
        tx
    });
    if let Err(error) = worker.send((action, tx)) {
        let _ = error.0.1.send_blocking(Err(StoreError::Unavailable));
    }
    rx
}

fn perform(action: Action) -> Result<Option<Credentials>, StoreError> {
    let mut command = Command::new("secret-tool");
    let input = match &action {
        Action::Load => {
            command.arg("lookup");
            None
        }
        Action::Clear => {
            command.arg("clear");
            None
        }
        Action::Save(credentials) => {
            if !credentials.valid() {
                return Err(StoreError::Invalid);
            }
            command.args(["store", "--label=LINE GTK startup login"]);
            Some(serde_json::to_vec(credentials).map_err(|_| StoreError::Invalid)?)
        }
    };
    command.args([
        "application",
        "dev.linegtk.LineGtk",
        "purpose",
        "startup-login",
    ]);
    let (success, output) = execute(command, input, Duration::from_secs(45))?;
    if !success {
        // 見つからない場合もlookupは終了コード1。サービス障害との区別はUIで不要。
        return Err(StoreError::Failed);
    }
    if matches!(action, Action::Load) {
        let text = String::from_utf8(output).map_err(|_| StoreError::Invalid)?;
        if text.trim().is_empty() {
            return Ok(None);
        }
        let credentials: Credentials =
            serde_json::from_str(&text).map_err(|_| StoreError::Invalid)?;
        if !credentials.valid() {
            return Err(StoreError::Invalid);
        }
        Ok(Some(credentials))
    } else {
        Ok(None)
    }
}

/// Key for the encrypted message cache, created on first use and kept in the
/// Secret Service. `None` when no keystore is usable; the sidecar then stores
/// plaintext as before instead of blocking startup.
pub fn cache_key() -> Option<String> {
    let attributes = [
        "application",
        "dev.linegtk.LineGtk",
        "purpose",
        "message-cache-key",
    ];
    let mut lookup = Command::new("secret-tool");
    lookup.arg("lookup").args(attributes);
    if let Ok((true, output)) = execute(lookup, None, Duration::from_secs(45))
        && let Ok(text) = String::from_utf8(output)
        && valid_cache_key(text.trim())
    {
        return Some(text.trim().to_string());
    }
    let key = random_cache_key()?;
    let mut store = Command::new("secret-tool");
    store
        .args(["store", "--label=LINE GTK message cache key"])
        .args(attributes);
    match execute(
        store,
        Some(key.clone().into_bytes()),
        Duration::from_secs(45),
    ) {
        Ok((true, _)) => Some(key),
        _ => None,
    }
}

fn valid_cache_key(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn random_cache_key() -> Option<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn execute(
    mut command: Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<(bool, Vec<u8>), StoreError> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| StoreError::Unavailable)?;
    let stdin = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let (Some(mut stdin), Some(input)) = (stdin, input) {
            stdin.write_all(&input)
        } else {
            Ok(())
        }
    });
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(stdout) = stdout {
            stdout.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
        }
        Ok::<_, std::io::Error>(bytes)
    });
    let deadline = Instant::now() + timeout;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status.success()),
            Err(_) => break Err(StoreError::Failed),
            Ok(None) if Instant::now() >= deadline => break Err(StoreError::Timeout),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    // タイムアウト時もパイプを閉じ、待機中の入出力スレッドを回収する。
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let written = writer.join().map_err(|_| StoreError::Failed)?;
    let output = reader
        .join()
        .map_err(|_| StoreError::Failed)?
        .map_err(|_| StoreError::Failed)?;
    let success = result?;
    if success && written.is_err() {
        return Err(StoreError::Failed);
    }
    if output.len() > LIMIT {
        return Err(StoreError::Invalid);
    }
    Ok((success, output))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_cache_keys_are_valid_and_unique() {
        let a = random_cache_key().unwrap();
        assert!(valid_cache_key(&a));
        assert_ne!(a, random_cache_key().unwrap());
        assert!(!valid_cache_key("short"));
        assert!(!valid_cache_key(&"g".repeat(64)));
    }
    #[test]
    fn secret_is_only_transferred_over_stdin() {
        let mut command = Command::new("/bin/cat");
        command.arg("-");
        assert!(!format!("{command:?}").contains("試験用パスワード"));
        let (success, bytes) = execute(
            command,
            Some("試験用パスワード".as_bytes().to_vec()),
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(success);
        assert_eq!(bytes, "試験用パスワード".as_bytes());
    }
    #[test]
    fn unavailable_and_stalled_stores_fail_without_falling_back_to_a_file() {
        assert_eq!(
            execute(
                Command::new("/存在しないキーストア"),
                None,
                Duration::from_millis(30)
            ),
            Err(StoreError::Unavailable)
        );
        let mut command = Command::new("/bin/sleep");
        command.arg("2");
        assert_eq!(
            execute(command, None, Duration::from_millis(30)),
            Err(StoreError::Timeout)
        );
    }
    #[test]
    fn excessive_secret_service_output_is_rejected() {
        let mut command = Command::new("/usr/bin/head");
        command.args(["-c", "17000", "/dev/zero"]);
        assert_eq!(
            execute(command, None, Duration::from_secs(2)),
            Err(StoreError::Invalid)
        );
    }
}
