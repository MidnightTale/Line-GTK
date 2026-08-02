# Product

## Register

product

## Users

Linux desktop users who need their existing LINE conversations, groups, OpenChat rooms, media, and account features without running a web wrapper, Wine, or Android container. They use the app throughout the day and expect chat switching, media, notifications, and keyboard workflows to respond immediately. Thai and English are first-class interface languages.

## Product Purpose

Provide a native GTK4 and Libadwaita LINE client for Linux, backed by linejs. Success means the everyday LINE workflow is familiar, locally cached, reliable, and fast while remaining honest about unofficial protocol limitations.

## Brand Personality

Familiar, direct, dependable. The interface should feel like LINE adapted carefully to a Linux desktop, with concise copy and native platform behavior.

## Anti-references

- Web wrappers that feel slow or ignore desktop conventions.
- Decorative chat interfaces that compete with conversation content.
- Controls that look unlike either LINE or the surrounding Libadwaita system.
- Silent failures, fake loading states, and features that appear available but do not work.

## Design Principles

- Keep conversation content primary and controls immediately understandable.
- Preserve familiar LINE concepts while using standard GTK affordances.
- Show cached content immediately, then reconcile network updates in the background.
- Make unavailable or experimental behavior explicit and recoverable.
- Keep per-chat preferences private, local, and reversible.

## Accessibility & Inclusion

Target WCAG AA contrast where theme colors are under app control. Follow the system light or dark theme, support reduced motion through the existing animation preference, retain visible focus and keyboard paths, avoid color-only state, and keep Thai and English layouts readable at the supported font scales.
