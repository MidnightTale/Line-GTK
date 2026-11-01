# Group mentions and reply navigation

In a group chat, type `@` followed by a member's name. Select a result with
Up/Down and Enter or Tab, or click it. Escape dismisses the picker. Duplicate
names include member IDs so they can be distinguished. Failed member requests
show a clickable retry row. Typing a name without selecting a member sends
ordinary text. Select **All members** (also found by typing `@all`) to tag
everyone in the group. The selected label is `@all`; typing it without selecting
the picker result sends ordinary text.

Selected mentions use the theme accent color, bold text, and an underline. Editing inside a selected name drops
its mention identity; edits before or after it retain and adjust the range.
Sending trims surrounding whitespace and adjusts mention offsets. Failed sends
restore mention identities when the same account and chat still have an empty
composer. Replies retain their quote alongside the mentions.

The sidecar converts Unicode scalar offsets from GTK to LINE's UTF-16 offsets,
encodes `contentMetadata.MENTION`, and preserves it through all send fallback
paths. Received mention metadata is bounded and validated before rendering.
Individual and all-member mentions can be sent and received. The all-member
picker result remains available while member lookup is loading or has failed.
Member lists are fetched lazily for the current group and discarded when
switching chats or accounts.

Incoming mentions of the signed-in account use the normal desktop notification
pipeline with “{sender} mentioned you in {group}” as the title. Notification and
sound preferences, chat muting, and active-conversation suppression still apply.
Clicking the notification opens the chat.

Click an individual mention in a received message to open that user's profile.
Text messages use native GTK links, which also support Tab and Enter. Message
bodies do not place a text cursor or start a selection when clicked. Right-click
a message (or use Menu / Shift+F10 with its bubble focused) and choose **Copy**
to copy the whole message, or **Select and copy…** to select part of its original
text in a separate dialog. The dialog supports Ctrl+C, **Copy selection**, and
**Copy all**, including the fallback text for custom LINE emoji.

Clicking a reply quote scrolls to its original message and briefly highlights it.
If necessary, the client loads older history, up to 100 pages (5,000 messages).
Missing originals produce a translated notice. Account, chat, and request
generation changes cancel navigation.

## Validation

Automated regression coverage includes Unicode offsets and surrogate boundaries,
malformed/overlapping metadata, duplicate member names, nonfriend members,
account changes during member lookup, all three send paths with reply metadata,
composer range edits, mention detection, and reply navigation targeting and
isolation. Run the repository's Rust and Deno checks and build the sidecar.

A logged-in manual check is still required before release:

- Exchange mentions with an official LINE client, including emoji before names.
- Check picker focus, keyboard/mouse selection, retry, and failed-send recovery.
- Check mention notifications with muted, active, and background conversations.
- Open mentioned profiles with the mouse and keyboard, including messages with
  custom LINE emoji. Check whole-message and partial copying with multiline text.
- Open reply originals in the current window, older history, and unavailable cases.
