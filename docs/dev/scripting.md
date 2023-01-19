
# Scripting - Flow Diagrams

1. [High level overview](#high-level-overview)
    1. [Notes](#notes)
2. [Script Execution](#script-execution)
3. [Nonce Management](#nonce-management)

## High level overview

```mermaid
graph TD;
    ScriptArgs::run_script-->ScriptArgs::compile;
    ScriptArgs::compile-->A{ScriptArgs::execute};
    A-- "(resume || verify) && !broadcast" -->B{ScriptArgs::resume_deployment};
    A-- "broadcast" -->ScriptArgs::handle_broadcastable_transactions;

    B-- multi-chain --> MultiChainSequence::load
    B-- single-chain --> ScriptArgs::resume_single_deployment
    ScriptArgs::resume_single_deployment--> ScriptSequence::load
    ScriptSequence::load--> receipts::wait_for_pending
    receipts::wait_for_pending -- resume --> ScriptArgs::send_transactions

    MultiChainSequence::load --> ScriptArgs::multi_chain_deployment

    ScriptArgs::multi_chain_deployment-- "resume[..]" --> receipts::wait_for_pending
    ScriptArgs::multi_chain_deployment-- "resume[..]" --> ScriptArgs::send_transactions
    ScriptArgs::multi_chain_deployment-- "broadcast[..]" --> ScriptArgs::send_transactions

    ScriptArgs::send_transactions--verify-->ScriptArgs::verify_contracts


    ScriptArgs::handle_broadcastable_transactions-->ScriptConfig::collect_rpcs;
    ScriptConfig::collect_rpcs-->ScriptArgs::create_script_sequences
    ScriptArgs::create_script_sequences--"[..]"-->C{Skip onchain simulation};
    C--yes-->D{"How many sequences?"};
    C--no-->ScriptArgs::fills_transactions_with_gas;
    ScriptArgs::fills_transactions_with_gas-->ScriptArgs::onchain_simulation;
    ScriptArgs::onchain_simulation-->D;

    D--"+1 seq"-->MultiChainSequence::new
    D--"1 seq"-->ScriptSequence::new

    MultiChainSequence::new-->ScriptArgs::multi_chain_deployment
    ScriptSequence::new-->ScriptArgs::single_deployment
    ScriptArgs::single_deployment-->ScriptArgs::send_transactions

```

### Notes
1) `[..]` - concurrently executed

2) The bit below does not actually influence the state initially defined by `--broadcast`. It only happens because there might be private keys declared inside the script that need to be collected again. `--resume` only resumes **publishing** the transactions, nothing more!

```mermaid
graph TD;
ScriptArgs::execute-- "(resume || verify) && !broadcast" -->ScriptArgs::resume_deployment;
```
3) `ScriptArgs::execute` executes the script, while `ScriptArgs::onchain_simulation` only executes the broadcastable transactions collected by `ScriptArgs::execute`.



## Script Execution
```mermaid
graph TD;
subgraph ScriptArgs::execute
a[*]-->ScriptArgs::prepare_runner
subgraph ::prepare_runner
ScriptArgs::prepare_runner--fork_url-->Backend::spawn;
ScriptArgs::prepare_runner-->Backend::spawn;
end
Backend::spawn-->
ScriptRunner-->ScriptRunner::setup;
subgraph ::setup
ScriptRunner::setup--libraries-->Executor::deploy;
ScriptRunner::setup--ScriptContract-->Executor::deploy;
Executor::deploy--"setUp()"-->Executor::call_committing;
end
subgraph ::script
Executor::call_committing-->ScriptRunner::script;
ScriptRunner::script--"run()"-->Executor::call;
end
end
Executor::call-. BroadcastableTransactions .->ScriptArgs::handle_broadcastable_transactions;

```


## Nonce Management

During the first execution stage on `forge script`, foundry has to adjust the nonce from the sender to make sure the execution and state are as close as possible to its on-chain representation.

Making sure that `msg.sender` is our signer when calling `setUp()` and `run()` and that its nonce is correct (decreased by one on each call) when calling `vm.broadcast` to create a contract.

We skip this, if the user hasn't set a sender and they're using the `Config::DEFAULT_SENDER`.


```mermaid
graph TD

    ScriptRunner::setup-->default_foundry_caller-deployScript;
    default_foundry_caller-deployScript-->user_sender-deployLibs;
    user_sender-deployLibs-->Contract.setUp;
    Contract.setUp-->A0{Executor::call};
    A0-->vm.broadcast;
    A0-->vm.startBroadcast;
    A0-->vm.getNonce;

    vm.broadcast--> A{cheatcode.corrected_nonce}
    vm.startBroadcast-->A
    vm.getNonce-->A

    A--true-->continue_setUp;
    A--false-->B[sender_nonce=-1];
    B-->C[cheatcode.corrected_nonce=true];
    C-->continue_setUp;
    continue_setUp-->end_setUp;
    end_setUp-->D{cheatcode.corrected_nonce}
    D--true-->E[cheatcode.corrected_nonce=false];
    D--false-->F[sender_nonce=initial_nonce+predeployed_libraries_count];
    E-->ScriptRunner::script;
    F-->ScriptRunner::script;
    ScriptRunner::script-->Contract.run;
    Contract.run-->G{Executor::call};
    G-->H[vm.broadcast];
    G-->I[vm.startBroadcast];
    G-->J[vm.getNonce];

    H--> K{cheatcode.corrected_nonce}
    I-->K
    J-->K

    K--true-->continue_run;
    K--false-->L[sender_nonce=-1];
    L-->M[cheatcode.corrected_nonce=true];
    M-->continue_run;
    continue_run-->end_run;
```