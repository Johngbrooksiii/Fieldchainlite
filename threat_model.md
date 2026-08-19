---

## THREAT_MODEL.md
```markdown
# Threat Model

## Assets protected

- Integrity of the submission chain (tamper‑evident).
- Ordering of records (monotonic counter).
- Detection of duplicate or conflicting submissions.

## Assumptions

- The file system provides durable writes after `fsync`.
- The system clock is not critical (counter is monotonic).
- Attacker cannot modify the binary or bypass the verification logic.
- The storage medium is not maliciously altered while the software is running.

## Non‑goals

- Protection against physical device theft or full OS compromise.
- Confidentiality of submission data.
- Protection against rollback of the entire chain (if an attacker can restore old snapshots, the chain can be reverted).
- Network security (no TLS used for local operations).

