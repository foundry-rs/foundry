interface Unknown {
    function empty() external;
    function lookup() external returns (uint256);
    function lookupMultipleValues()
        external
        returns (uint256, uint256, uint256, uint256, uint256);

    function doSomething() external;
    function doSomethingElse() external;

    function handleError() external;
}

contract TryStatement {
    Unknown unknown;

    function test() external {
        try unknown.empty() {} catch {}

        try unknown.lookup() returns (uint256) {} catch Error(string memory) {}

        try unknown.lookup() returns (uint256) {}
            catch Error(string memory) {}
            catch (bytes memory) {}

        try unknown.lookup() returns (uint256) {} catch (bytes memory) {}

        try unknown.empty() {
            unknown.doSomething();
        } catch {
            unknown.handleError();
        }

        try unknown.empty() {
            unknown.doSomething();
        }
            catch Error(string memory) {}
            catch Panic(uint256) {}
        catch {
            unknown.handleError();
        }

        try unknown.lookupMultipleValues() returns (
            uint256, uint256, uint256, uint256, uint256
        ) {}
            catch Error(string memory) {}
            catch {}

        try unknown.lookupMultipleValues() returns (
            uint256, uint256, uint256, uint256, uint256
        ) {
            unknown.doSomething();
        } catch Error(string memory) {
            unknown.handleError();
        } catch {}

        // comment1
        try /* comment2 */ unknown.lookup() // comment3
        returns (
            uint256 // comment4
        ) {} // comment5
            catch { /* comment6 */ }

        // comment7
        try unknown.empty() {
            // comment8
            unknown.doSomething();
        } /* comment9 */ catch /* comment10 */ Error(string memory) {
            unknown.handleError();
        } catch /* comment11 */ Panic(uint256) {
            unknown.handleError();
        } catch {}
    }
}
