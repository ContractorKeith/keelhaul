# Key slots, unlock and secrets

Governs `src/crypto/slot.rs`, `src/repo/open.rs`, `src/repo/blobs/writer.rs`, `src/secret.rs`.

KDF cost parameters in a key slot are attacker-controlled and are validated before Argon2 is constructed. Writers refuse empty passwords; readers still accept them, so a repository made by another implementation still opens.

A slot's master-keyed `check` tag is redundant: authenticating config is what authorises opening, so a slot that authenticates config opens despite a bad tag. Unlock is independent of config, so audits can still report config damage: an unwrapped key is kept for the audit while `open` still authenticates and parses config before accepting it. A sole slot, or slots that agree on one master key, identify config damage; a foreign slot disagrees with the other slots as well as failing config authentication.

Backup read workers get derived subkeys; the master key stays private to the repository. `Secret` deliberately has no `Debug`, so secret bytes can never reach diagnostics.

Init: config wins the create-once race before any slot is stored, and an existing repository is never reset. A crash between the config and slot writes leaves an unusable repository that must be deleted by hand; tracked in #59.
