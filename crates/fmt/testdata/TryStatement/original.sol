interface Unknown {
    function empty() external;
    function lookup() external returns(uint256);
    function lookupMultipleValues() external returns (uint256, uint256, uint256, uint256, uint256);

    function doSomething() external;
    function doSomethingElse() external;

    function handleError() external;
}

contract TryStatement {
    Unknown unknown;

    function test() external {
        try unknown.empty() {} catch {}

        try unknown.lookup() returns (uint256) {} catch Error(string memory) {}

        try unknown.lookup() returns (uint256) {} catch Error(string memory) {} catch (bytes memory) {}

    try unknown
        .lookup() returns   (uint256
                ) {
                } catch ( bytes  memory ){}

        try unknown.empty() {
            unknown.doSomething();
        } catch {
            unknown.handleError();
        }

        try unknown.empty() {
            unknown.doSomething();
        } catch Error(string memory) {}
        catch Panic(uint) {}
        catch {
            unknown.handleError();
        }

        try unknown.lookupMultipleValues() returns (uint256, uint256, uint256, uint256, uint256) {} catch Error(string memory) {} catch {}

        try unknown.lookupMultipleValues() returns (uint256, uint256, uint256, uint256, uint256) {
            unknown.doSomething();
        }
        catch Error(string memory) {
             unknown.handleError();
        }
        catch {}

        // comment1
        try /* comment2 */ unknown.lookup() // comment3
        returns (uint256) // comment4
        {} // comment5
        catch /* comment6 */ {}

        // comment7
        try unknown.empty() { // comment8
            unknown.doSomething();
        } /* comment9 */ catch /* comment10 */ Error(string memory) {
            unknown.handleError();
        } catch Panic /* comment11 */ (uint) {
            unknown.handleError();
        } catch {}
    }

    function test_multiParam() {
        Mock mock = new Mock();

        try mock.add(2, 3) {
            revert();
        } catch (bytes memory err) {
            require(keccak256(err) == keccak256(ERROR_MESSAGE));
        }
    }

    function test_multiComment() {
        try vm.envString("API_KEY") returns (string memory) {
            console2.log("Forked Ethereum mainnet");
            // Fork mainnet at a specific block for consistency
            vm.createSelectFork(vm.rpcUrl("mainnet"), 21_900_000);
            // do something
        } catch /* sadness */ {
            // more sadness
            revert();
        }
    }
}
