.PHONY: fmt-testdata

fmt-testdata:
	@FOLDER=$(shell dirname "$0")/fmt/testdata/prettier-plugin-solidity;\
	if [ ! -d $$FOLDER/.git ] ; then git clone --depth 1 --recursive https://github.com/prettier-solidity/prettier-plugin-solidity $$FOLDER;\
	else cd $$FOLDER; git pull --recurse-submodules; fi
