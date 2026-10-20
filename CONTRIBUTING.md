# Contributing

Changes land through a pull request. Nothing is pushed straight to `main`, and
every pull request needs a maintainer review before it merges.

## Secrets

Never commit credentials, `.env` files, signing keys, or licence seeds. The
secret scan runs on every pull request and blocks the merge when it finds one.
If you do commit one, treat it as leaked: revoke it first, then clean up the
history.

## Path safety

`uncached-core` owns path validation and secure cleanup. Changes to those
modules need a test that shows a path outside the allowed roots is refused, and
they need a maintainer review even when the diff looks small.

## Contract

Serialized types in `contract::v1` are public. Changing a field or a variant is
a breaking change: update `CONTRACT_V1.md` in the same pull request and say so
in the description.
