# Releases and support

## Version numbers

Liberarr follows [Semantic Versioning](https://semver.org). While the version starts with `0`:

- A new **minor** version (`0.2` to `0.3`) may change commands, settings or the data format. Each such change is listed first in the release notes, with what you must do.
- A **patch** version (`0.2.0` to `0.2.1`) only fixes bugs. It never needs an action from you.

## Your data across updates

- The database is upgraded by itself the first time a new version opens it.
- Going back to an older version is not supported: an older binary refuses to open a database written by a newer one. Copy your `LIBERARR_HOME` folder before a minor update if you want a way back.
- The original audio files are never changed by an update.
- A setting of the `.env` file is never renamed or removed in a patch version. When one changes in a minor version, the release notes say so.

## Release notes

Each [release](https://github.com/hocinehacherouf/liberarr/releases) lists its changes, breaking changes first. To check that a download was built by this repository, see [Install](install.md).

## Support

Liberarr is maintained by one person in their spare time.

- Only the latest release is supported. Before you report a bug, check that it still happens with it.
- Bugs and questions go to the [issues](https://github.com/hocinehacherouf/liberarr/issues). There is no promised answer time. Security reports are different: see the [security policy](../SECURITY.md).
- Supported systems: macOS (Apple silicon and Intel) and Linux (x86-64 and ARM64). Windows is not supported.
- AI providers change their APIs without notice. The maintainer runs Mistral and the `claude` CLI against the real services. The OpenAI and Anthropic API adapters are tested against the providers' documentation and a mock server only: see [AI providers](ai-providers.md#tested-providers).
