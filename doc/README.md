# Documentation (`doc`)

Solidity documentation generator. It parses the source code and generates an mdbook
based on the parse tree and [NatSpec comments](https://docs.soliditylang.org/en/v0.8.17/natspec-format.html).

## Architecture

The entrypoint for the documentation module is the `DocBuilder`.
The `DocBuilder` generates the mdbook in 3 phases:

1. Parse

In this phase, builder invokes 2 parsers: [solang parser](https://github.com/hyperledger-labs/solang) and internal `Parser`. The solang parser produces the parse tree based on the source code. Afterwards, the internal parser walks the parse tree by implementing the `Visitor` trait from the `fmt` crate and saves important information about the parsed nodes, doc comments.

Then, builder takes the output of the internal `Parser` and creates documents with additional information: the path of the original item, display identity, the target path where this document will be written.


2. Preprocess

The builder accepts an array of preprocessors which can be applied to documents produced in the `Parse` phase. The preprocessors can rearrange and/or change the array as well as modify the separate documents.

At the end of this phase, the builder maintains a possibly modified collection of documents.


3. Write

At this point, builder has all necessary information to generate documentation for the source code. It takes every document, formats the source file contents and writes/copies additional files that are required for building documentation.