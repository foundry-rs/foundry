---
title: "chore: refund the testnet deploy+verify deployer"
labels: P-normal, T-chore
---

The shared deployer for the nightly on-chain deploy + verify workflow is running low on at least one network. The tests may still be passing right now; this is raised early so the account can be topped up before they start failing.

Refund `{{ env.DEPLOYER }}` on:

```
{{ env.LOW_NETWORKS }}
```

Each entry is `NETWORK=current(min threshold)`. Thresholds are set at roughly twenty more runs at the measured per-run burn, so there is room to act. Monad testnet drains fastest at about 0.08 MON per run; the L2s are effectively free and should only appear here if something has gone wrong.

The key lives in the `FUNDED_TESTNET_PKEY` secret and holds testnet funds only. Public faucets cover every network involved.

Check the [deploy + verify workflow page]({{ env.WORKFLOW_URL }}) for the full balance report.

This issue was raised by the workflow at `.github/workflows/test-deploy-verify.yml`.
