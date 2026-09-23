# IPC fuzz seeds

Each `.hex` file contains one wire input. The driver also seeds valid Packet,
Handshake, Navigate, and GPU submissions from typed Rust values, then mutates
both sets with the fixed `0x1a2b3c4d5e6f7890` seed. Novel decoder-result
signatures are minimized and retained in memory for the remainder of a run.

Run `ipc-fuzz --target decoders --fuzz-seconds 600` and repeat with
`--target dispatchers`. A panic writes its input in hex below
`target/ipc-fuzz-crashes/` for promotion to `fuzz/crashes/` after minimization.
