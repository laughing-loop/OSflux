# Security policy

OSflux runs as Administrator and erases drives, so security problems matter more here than in most tools. Thank you for reporting them responsibly.

## Reporting a vulnerability

**Please do not report security problems in public issues, pull requests or discussions.**

Report them privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. Only the maintainers can see the report.

Please include:

- what the problem is and what an attacker could do with it;
- the steps to reproduce it, with the OSflux version (`osflux --version`) or commit, the operating system, and the image and drive involved, where they matter;
- any proof of concept, logs or screenshots.

## What to expect

- We aim to acknowledge a report within 5 working days.
- We will keep you informed while we investigate and fix it, and tell you when a fix is released.
- We will credit you in the release notes, unless you would rather not be named.
- Please give us a reasonable time to release a fix before you disclose the problem publicly. We will agree a date with you, normally within 90 days of the report.

## What counts

We especially want to hear about anything that could make OSflux:

- write to a drive other than the one the user confirmed, or write without a valid confirmation;
- accept a confirmation that is stale, reused, or meant for another drive or image;
- write to a fixed disk, or a drive the safety chain should have blocked;
- run code or escalate privileges through a crafted image, answer file, file name or state folder;
- leave a written stick different from the image without reporting it;
- lose, alter or skip entries in the audit log.

Bugs that do not have a security impact, such as a stick that does not boot or a crash on a malformed image with no further effect, can be reported as ordinary issues.

## Supported versions

OSflux is in early development. Security fixes go into the latest release and `main`; older releases are not patched.
