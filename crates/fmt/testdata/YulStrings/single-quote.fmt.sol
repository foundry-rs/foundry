// config: quote_style = "single"
contract Yul {
    function test() external {
        assembly {
            let a := 'abc'
            let b := 'abc'
            let c := hex'deadbeef'
            let d := hex'deadbeef'
            let e := 0xffffffffffffffffffffffffffffffffffffffff
            datacopy(0, dataoffset('runtime'), datasize('runtime'))
            return(0, datasize('runtime'))
        }
    }
}
