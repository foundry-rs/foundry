
# Scripting - Flow Diagrams

1. [High level overview](#high-level-overview)
    1. [Notes](#notes)
2. [Script Execution](#script-execution)

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

