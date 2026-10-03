# Linux utility bootstrap filter correction

On Boat, the existing ignored real utility sandbox integration test failed with UnexpectedEof before the bootstrap acknowledgment. A native strace showed user/UID/GID/mount namespaces and seccomp installation succeeded, followed by denied getrandom/openat and even denied diagnostic writes. The clone flags guard used a false jump of four instructions, landing every non-clone syscall directly on its deny return. Five skips the complete guard and reaches the existing explicit allowlist. No syscall permission is added and the child still has no filesystem/network/process creation authority.

Source fix is pending the whole Process/workspace/doc and actual browser image-resource gate. Existing fail-closed behavior was retained during diagnosis. Linux sandbox tests must exercise the actual child, not the unit-test decoder fallback.
