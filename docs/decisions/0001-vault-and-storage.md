# 0001: Vault key lifecycle and core storage

## Decision

- **Envelope keys.** One random 32-byte vault key encrypts every credential. The vault key is wrapped, never derived directly from the password, so the password can change and DPAPI can be added or removed without re-encrypting records.
- **Wrapping.** Password wrap: Argon2id (64 MiB, t=3, p=1, 16-byte salt) then XChaCha20-Poly1305. DPAPI wrap: `CryptProtectData` scoped to the current Windows user. Either wrap unlocks; KDF params live in the header so they can be raised later.
- **Records.** Username and password are sealed together with XChaCha20-Poly1305, fresh 24-byte `OsRng` nonce per seal. AAD binds `umbrella/credential/v1`, the canonical origin (`scheme://host:port`, port always explicit) and the record ID, so a ciphertext moved to another record or origin fails to open.
- **Origins.** Parsed with the `url` crate. Only `http` and `https`. Match is exact equality on scheme, host and port. No substring, suffix or registrable-domain matching.
- **Storage.** `redb` (pure Rust, ACID, no C toolchain). Session and vault use separate files so the vault can be deleted or backed up alone. Values are JSON for debuggability; the vault file holds only ciphertext and public header data.
- **Session writes.** `browser-core` stays runtime-free: a `Debouncer` says when a save is due (500 ms after the last change) and the UI flushes it on a worker thread and on close.

## Council

- **Systems:** Argon2id at 64 MiB costs ~100-300 ms once per unlock, off the UI thread. Per-record decryption is microseconds, so listing usernames for one origin is cheap. redb keeps startup to one file open.
- **Security:** Wrong password fails at the key-unwrap tag, so there is no separate verifier to leak. Keys and decrypted payloads are `Zeroizing`; payload encoding is pre-sized so no reallocation leaves copies behind. `Debug` impls never print secrets. Origins stay in clear so lookup needs no decryption; accepted because history already records visited origins.
- **UX:** Fill prompts need usernames only; passwords are revealed at fill time and dropped right after. Idle lock is a timestamp check the UI drives, no timer thread.
- **Skeptic:** Corrupt or newer-version files must fail loudly, not reset silently. DPAPI blobs do not roam to other machines or users, so the password wrap stays the recovery path; a DPAPI-only vault is not allowed.

## Deferred

- Hashing origins at rest. Reopen if the vault file is meant to be shared or synced.
- Raising Argon2 params on unlock when the stored ones are below the current default.
