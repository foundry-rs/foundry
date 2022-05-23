#!/bin/sh
# foundryup shell setup
# affix colons on either side of $PATH to simplify matching
case ":${PATH}:" in
    *:"{foundry_bin}":*)
        ;;
    *)
        # Prepending path in case a cargo installed version needs to be overridden
        export PATH="{foundry_bin}:$PATH"
        ;;
esac
