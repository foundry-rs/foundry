//! CLI tests for the Azure Key Vault wallet signer.

use super::*;

// tests that `--azure` requires a key identifier
casttest!(wallet_address_azure_requires_key_id, |_prj, cmd| {
    cmd.unset_env("AZURE_KEY_VAULT_KEY_ID");
    cmd.args(["wallet", "address", "--azure"]).assert_failure().stderr_eq(str![[r#"
Error: AZURE_KEY_VAULT_KEY_ID environment variable is required for signer

"#]]);
});

// tests that `--azure` reports a build without Azure Key Vault support
#[cfg(not(feature = "azure-key-vault"))]
casttest!(wallet_address_azure_unsupported, |_prj, cmd| {
    cmd.env("AZURE_KEY_VAULT_KEY_ID", "https://my-vault.vault.azure.net/keys/my-key");
    cmd.args(["wallet", "address", "--azure"]).assert_failure().stderr_eq(str![[r#"
Error: foundry was not built with support for Azure Key Vault signer

"#]]);
});

// tests that `--azure` rejects an identifier that is not a Key Vault key before any request
#[cfg(feature = "azure-key-vault")]
casttest!(wallet_address_azure_rejects_non_key_identifier, |_prj, cmd| {
    cmd.unset_env("AZURE_CLIENT_SECRET");
    cmd.unset_env("AZURE_FEDERATED_TOKEN_FILE");
    cmd.env("AZURE_KEY_VAULT_KEY_ID", "https://my-vault.vault.azure.net/secrets/my-secret");
    cmd.args(["wallet", "address", "--azure"]).assert_failure().stderr_eq(str![[r#"
Error: invalid key identifier `https://my-vault.vault.azure.net/secrets/my-secret`, expected `https://<vault>.vault.azure.net/keys/<name>[/<version>]`: not in keys collection

"#]]);
});
