(function() {
    var implementors = Object.fromEntries([["foundry_evm_core",[["impl&lt;'db, I: <a class=\"trait\" href=\"foundry_evm_core/trait.InspectorExt.html\" title=\"trait foundry_evm_core::InspectorExt\">InspectorExt</a>&gt; Evm for <a class=\"struct\" href=\"foundry_evm_core/evm/struct.FoundryEvm.html\" title=\"struct foundry_evm_core::evm::FoundryEvm\">FoundryEvm</a>&lt;'db, I&gt;"],["impl&lt;DB, I, P&gt; Evm for <a class=\"enum\" href=\"foundry_evm_core/either_evm/enum.EitherEvm.html\" title=\"enum foundry_evm_core::either_evm::EitherEvm\">EitherEvm</a>&lt;DB, I, P&gt;<div class=\"where\">where\n    DB: Database,\n    I: Inspector&lt;EthEvmContext&lt;DB&gt;&gt; + Inspector&lt;OpContext&lt;DB&gt;&gt;,\n    P: PrecompileProvider&lt;EthEvmContext&lt;DB&gt;, Output = InterpreterResult&gt; + PrecompileProvider&lt;OpContext&lt;DB&gt;, Output = InterpreterResult&gt;,</div>"]]]]);
    if (window.register_implementors) {
        window.register_implementors(implementors);
    } else {
        window.pending_implementors = implementors;
    }
})()
//{"start":57,"fragment_lengths":[836]}