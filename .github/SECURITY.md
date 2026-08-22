# Security Policy

## Supported versions

Only the latest release is supported. There are no backport branches; fixes ship
as a new release.

## Reporting a vulnerability

Please report vulnerabilities privately via GitHub's private vulnerability
reporting: <https://github.com/montezuma-p/harken/security/advisories/new>.
Do not open a public issue for a security problem.

You can expect an acknowledgement within a few days. If the report is accepted,
a fix is released and the advisory is published once users can upgrade.

## Scope notes

harken is an offline CLI: transcription never touches the network. The
security-relevant surface is:

- parsing of untrusted input — WhatsApp export zips and audio files
  (opus/ogg/mp3/m4a/wav decoding),
- the model download path (`https://huggingface.co`), the only network access,
- the vendored whisper.cpp sources compiled into the binary.

Vulnerabilities in whisper.cpp itself should also be reported upstream, but a
report here is welcome so the vendored pin can be bumped.
