---
title: "bug: deploy + verify workflow failed"
labels: P-normal, T-bug
---

The nightly on-chain deploy + verify workflow has failed. This exercises `forge create` followed by `forge verify-contract` against Hoodi, Sepolia, Base Sepolia, Monad testnet and Robinhood testnet, using Etherscan, Sourcify and Blockscout.

The most common cause is the shared deployer account running out of testnet funds; the workflow logs its balance on every network before running the tests. Other likely causes are a testnet RPC being down, a verifier service being unavailable, or Etherscan rate limiting.

Check the [deploy + verify workflow page]({{ env.WORKFLOW_URL }}) for details.

This issue was raised by the workflow at `.github/workflows/test-deploy-verify.yml`.
