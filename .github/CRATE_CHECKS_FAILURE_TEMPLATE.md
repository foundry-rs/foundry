---
title: "bug: nightly crate-checks workflow failed"
labels: P-normal, T-bug
---

The nightly crate-checks workflow (`cargo hack check`) has failed. This means one or more crates don't compile in isolation — likely a missing dependency or feature flag masked by Cargo's workspace feature unification.

Check the [crate-checks workflow page]({{ env.WORKFLOW_URL }}) for details.

This issue was raised by the workflow at `.github/workflows/crate-checks.yml`.
