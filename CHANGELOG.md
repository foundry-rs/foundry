## Pre 1.0

### Important note for users

Multiple breaking changes will occur so Semver can be followed as soon as Foundry 1.0 is released. They will be listed here, along with the updates needed for your projects.

If you need a stable Foundry version, we recommend using the latest pinned nightly of May 2nd, locally and on your CI.

To use the latest pinned nightly locally, use the following command:

```
foundryup --version nightly-e15e33a07c0920189fc336391f538c3dad53da73
````

To use the latest pinned nightly on your CI, modify your Foundry installation step to use an specific version:

```
- name: Install Foundry
  uses: foundry-rs/foundry-toolchain@v1
  with:
    version: nightly-e15e33a07c0920189fc336391f538c3dad53da73
```

### Breaking changes

- [expectEmit](https://github.com/foundry-rs/foundry/pull/4920) will now only work for the next call.
- expectCall will now only work if the call(s) are made exactly after the cheatcode is invoked.
- expectRevert will now work if the next call does revert, instead of expecting a revert during the whole test.
- [precompiles will not be compatible with all cheatcodes](https://github.com/foundry-rs/foundry/pull/4905).
- The difficulty and prevrandao cheatcodes now [fail if not used with the correct EVM version](https://github.com/foundry-rs/foundry/pull/4904).
- The default EVM version will be Shanghai. If you're using an EVM chain which is not compatible with [EIP-3855](https://eips.ethereum.org/EIPS/eip-3855) you need to change your EVM version. See [Matt Solomon's thread]([https://twitter.com/msolomon44](https://twitter.com/msolomon44/status/1656411871635972096)) for more information.
