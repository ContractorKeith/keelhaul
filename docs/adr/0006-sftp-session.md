# SFTP: create-once by v3 RENAME, session lifecycle, ssh options

Extends ADR 0002. Governs `src/backend/sftp/`.

**Create-once.** Writes upload to a temporary name and publish with plain v3 RENAME, which refuses an existing destination, unlike POSIX rename or the posix-rename extension. That holds across independent clients, not just inside this process's mutex. When RENAME fails, the stored object is compared: identical bytes are a successful retry, different bytes are a conflict.

**Session lifecycle.** A valid STATUS reply is an operation result and leaves the stream usable; only a protocol violation (`InvalidData`) aborts the session and kills `ssh`. Handles from an aborted session cannot be closed on a fresh connection, so `close` refuses them. Absence is a successful probe: `open_optional` maps NotFound to `None` without reconnecting, so probing every new object does not cost a new connection.

**Special files.** `read_file` STATs before OPEN and rejects anything that is not a regular file; opening a FIFO could block the server.

**ssh options.** User arguments from `KEELHAUL_SSH_ARGS` come before the defaults (`ServerAliveInterval=15`, `ServerAliveCountMax=4`, `BatchMode=yes`), because ssh uses the first value it sees, so explicit `-o` options override ours. The SFTP test peer (`tests/common/sftp_server.py`) is a deliberately small, independent v3 implementation, so client contract tests are not checked against the client's own assumptions.
