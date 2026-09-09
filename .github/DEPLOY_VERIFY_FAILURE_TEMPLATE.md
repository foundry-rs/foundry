---
title: "bug: deploy + verify workflow failed"
labels: P-normal, T-bug
---

The nightly on-chain deploy + verify workflow has failed. This exercises `forge create` followed by `forge verify-contract` against Hoodi, Sepolia, Base Sepolia, Arbitrum Sepolia, Monad testnet and Robinhood testnet, using Etherscan, Sourcify and Blockscout.

The most common cause is the shared deployer running out of testnet funds. The deployer is `0x14459e92f32B68125525B5e3cdF3A239618e40D8`, held in the `FUNDED_TESTNET_PKEY` secret, and the workflow logs its balance on every network before running the tests. Monad testnet drains fastest, at roughly 0.08 MON per run. Top it up from a faucet to fix.

Other likely causes are a testnet RPC being down, a verifier service being unavailable, or Etherscan rate limiting.

Check the [deploy + verify workflow page]({{ env.WORKFLOW_URL }}) for details.

This issue was raised by the workflow at `.github/workflows/test-deploy-verify.yml`.
