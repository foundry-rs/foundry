interface I {
    enum Empty {

    }

    /// A modification applied to either `msg.sender` or `tx.origin`. Returned by `readCallers`.
    enum CallerMode
    {/// No caller modification is currently active.
        None
    }

    /// A modification applied to either `msg.sender` or `tx.origin`. Returned by `readCallers`.
    enum CallerMode2
    {/// No caller modification is currently active.
        None,/// No caller modification is currently active2.

        Some
    }

    function bar() public {

    }
}