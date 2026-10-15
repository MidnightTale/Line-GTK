use crate::protocol::ReaderRanges;
use std::collections::HashMap;

#[derive(Default)]
pub struct ReadReceipts {
    chats: HashMap<String, HashMap<String, Vec<(u128, u128)>>>,
}

fn message_id(value: &str) -> Option<u128> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn valid_reader(chat_mid: &str, user_mid: &str, self_mid: &str) -> bool {
    !self_mid.is_empty()
        && user_mid.starts_with('u')
        && user_mid != self_mid
        && (chat_mid.starts_with('c') || chat_mid.starts_with('r') || chat_mid == user_mid)
}

fn merge(ranges: &mut Vec<(u128, u128)>, incoming: (u128, u128)) -> bool {
    let previous = ranges.clone();
    ranges.push(incoming);
    ranges.sort_unstable();
    let mut merged: Vec<(u128, u128)> = Vec::new();
    for (start, end) in ranges.drain(..) {
        if let Some(last) = merged.last_mut()
            && start <= last.1.saturating_add(1)
        {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    *ranges = merged;
    *ranges != previous
}

impl ReadReceipts {
    pub fn clear(&mut self) {
        self.chats.clear();
    }

    pub fn apply_snapshot(
        &mut self,
        chat_mid: &str,
        readers: &[ReaderRanges],
        self_mid: &str,
    ) -> bool {
        let mut changed = false;
        for reader in readers {
            if !valid_reader(chat_mid, &reader.user_mid, self_mid) {
                continue;
            }
            for range in &reader.ranges {
                let (Some(start), Some(end)) = (
                    message_id(&range.start_message_id),
                    message_id(&range.end_message_id),
                ) else {
                    continue;
                };
                if start > end {
                    continue;
                }
                let ranges = self
                    .chats
                    .entry(chat_mid.into())
                    .or_default()
                    .entry(reader.user_mid.clone())
                    .or_default();
                changed |= merge(ranges, (start, end));
            }
        }
        changed
    }

    pub fn observe(&mut self, chat_mid: &str, user_mid: &str, id: &str, self_mid: &str) -> bool {
        if !valid_reader(chat_mid, user_mid, self_mid) {
            return false;
        }
        let Some(id) = message_id(id) else {
            return false;
        };
        let ranges = self
            .chats
            .entry(chat_mid.into())
            .or_default()
            .entry(user_mid.into())
            .or_default();
        let start = if chat_mid.starts_with('u') {
            0
        } else {
            // グループの参加前や未確認の古い履歴へ、既読範囲を広げない。
            match ranges.last() {
                Some((start, end)) if id >= *end => *start,
                Some(_) => return false,
                None => id,
            }
        };
        merge(ranges, (start, id))
    }

    pub fn count(&self, chat_mid: &str, id: &str, self_mid: &str) -> usize {
        let Some(id) = message_id(id) else {
            return 0;
        };
        self.chats
            .get(chat_mid)
            .map(|readers| {
                readers
                    .iter()
                    .filter(|(user, ranges)| {
                        valid_reader(chat_mid, user, self_mid)
                            && ranges.iter().any(|(start, end)| *start <= id && id <= *end)
                    })
                    .count()
            })
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(user: &str, start: &str, end: &str) -> ReaderRanges {
        serde_json::from_value(serde_json::json!({"userMid":user,"ranges":[{"startMessageId":start,"endMessageId":end}]})).unwrap()
    }

    #[test]
    fn individual_receipts_ignore_self_other_chats_and_pending_ids() {
        let mut receipts = ReadReceipts::default();
        assert!(!receipts.observe("u-peer", "u-self", "50", "u-self"));
        assert!(!receipts.observe("u-peer", "u-other", "50", "u-self"));
        assert!(receipts.observe("u-peer", "u-peer", "50", "u-self"));
        assert_eq!(receipts.count("u-peer", "49", "u-self"), 1);
        assert_eq!(receipts.count("u-peer", "51", "u-self"), 0);
        assert_eq!(receipts.count("u-peer", "pending-1", "u-self"), 0);
        assert!(!receipts.observe("u-peer", "u-peer", "40", "u-self"));
    }

    #[test]
    fn group_counts_are_per_reader_and_respect_gaps_and_large_ids() {
        let mut receipts = ReadReceipts::default();
        let id = "9007199254740993";
        receipts.apply_snapshot(
            "c-group",
            &[
                reader("u-one", id, "9007199254740995"),
                reader("u-one", id, "9007199254740995"),
                reader("u-two", id, id),
                reader("u-self", "0", "9999999999999999"),
            ],
            "u-self",
        );
        assert_eq!(receipts.count("c-group", "9007199254740992", "u-self"), 0);
        assert_eq!(receipts.count("c-group", id, "u-self"), 2);
        assert_eq!(receipts.count("c-group", "9007199254740994", "u-self"), 1);
        assert_eq!(receipts.count("c-other", id, "u-self"), 0);
        assert_eq!(receipts.count("c-group", "pending-1", "u-self"), 0);
    }

    #[test]
    fn late_snapshots_keep_newer_reads_and_new_members_do_not_read_old_history() {
        let mut receipts = ReadReceipts::default();
        receipts.apply_snapshot("c-group", &[reader("u-one", "100", "110")], "u-self");
        receipts.observe("c-group", "u-one", "120", "u-self");
        receipts.apply_snapshot("c-group", &[reader("u-one", "100", "105")], "u-self");
        assert_eq!(receipts.count("c-group", "120", "u-self"), 1);
        assert_eq!(receipts.count("c-group", "99", "u-self"), 0);
        receipts.observe("c-group", "u-two", "120", "u-self");
        assert_eq!(receipts.count("c-group", "110", "u-self"), 1);
        assert_eq!(receipts.count("c-group", "120", "u-self"), 2);
        receipts.apply_snapshot("c-group", &[reader("u-two", "110", "120")], "u-self");
        assert_eq!(receipts.count("c-group", "110", "u-self"), 2);
        receipts.clear();
        assert_eq!(receipts.count("c-group", "120", "u-self"), 0);
    }
    #[test]
    fn snapshot_intervals_keep_unconfirmed_gaps() {
        let mut receipts = ReadReceipts::default();
        receipts.apply_snapshot(
            "c-group",
            &[reader("u-one", "100", "110"), reader("u-one", "120", "130")],
            "u-self",
        );
        assert_eq!(receipts.count("c-group", "115", "u-self"), 0);
        receipts.observe("c-group", "u-one", "140", "u-self");
        assert_eq!(receipts.count("c-group", "115", "u-self"), 0);
        assert_eq!(receipts.count("c-group", "140", "u-self"), 1);
    }
}
