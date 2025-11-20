# Scripting

- [Scripting](#scripting)
  - [High level overview](#high-level-overview)
    - [Notes](#notes)
  - [Script Execution](#script-execution)
  - [Nonce Management](#nonce-management)

## High level overview

```mermaid
graph TD;
    ScriptArgs::run_script-->PreprocessedState::compile;
    PreprocessedState::compile-->A{PreExecutionState::execute};
    A-- "(resume || verify) && !broadcast" -->B{CompiledState::resume};
    A-- "broadcast" -->ScriptRunner::script;

    B-- multi-chain --> MultiChainSequence::load
    B-- single-chain --> ScriptSequence::load
    ScriptSequence::load--> BundledState::wait_for_pending
    BundledState::wait_for_pending -- resume --> BundledState::broadcast

    MultiChainSequence::load --> ScriptArgs::multi_chain_deployment

    ScriptArgs::multi_chain_deployment-- "resume[..]" --> BundledState::wait_for_pending
    ScriptArgs::multi_chain_deployment-- "resume[..]" --> BundledState::broadcast
    ScriptArgs::multi_chain_deployment-- "broadcast[..]" --> BundledState::broadcast

    BundledState::broadcast--verify-->BroadcastedState::verify


    ScriptRunner::script-->RpcData::from_transactions;
    RpcData::from_transactions-->FilledTransactionsState::bundle
    FilledTransactionsState::bundle--"[..]"-->C{Skip onchain simulation};
    C--yes-->D{"How many sequences?"};
    C--no-->PreSimulationState::fill_metadata;
    PreSimulationState::fill_metadata-->PreSimulationState::simulate_and_fill;
    PreSimulationState::simulate_and_fill-->D;

    D--"+1 seq"-->MultiChainSequence::new
    D--"1 seq"-->ScriptSequenceKind::Single

    MultiChainSequence::new-->ScriptSequenceKind::Multi
    ScriptSequenceKind::Single-->BundledState::broadcast
```

### Notes

1. `[..]` - concurrently executed

2. The bit below does not actually influence the state initially defined by `--broadcast`. It only happens because there might be private keys declared inside the script that need to be collected again. `--resume` only resumes **publishing** the transactions, nothing more!

```mermaid
graph TD;
PreExecutionState::execute-- "(resume || verify) && !broadcast" -->CompiledState::resume;
```

3. `PreExecutionState::execute` executes the script, while `PreSimulationState::simulate_and_fill` executes the broadcastable transactions collected by `ScriptRunner::script`.

## Script Execution

```mermaid
graph TD;
subgraph PreExecutionState::execute
a[*]-->ScriptConfig::get_runner_with_cheatcodes
subgraph ::prepare_runner
ScriptConfig::get_runner_with_cheatcodes--fork_url-->Backend::spawn;
ScriptConfig::get_runner_with_cheatcodes-->Backend::spawn;
end
Backend::spawn-->ScriptRunner-->ScriptRunner::setup;
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
Executor::call-. BroadcastableTransactions .->PreSimulationState::fill_metadata;
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
