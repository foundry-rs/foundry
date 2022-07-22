// config: quote_style = "preserve"
contract Yul {
    function test() external {
        assembly {
            let a := "abc"
            let b := 'abc'
            let c := "abc":u32
            let d := 'abc':u32
            let e := hex"deadbeef"
            let f := hex'deadbeef'
            let g := hex"deadbeef":u32
            let h := hex'deadbeef':u32
            datacopy(0, dataoffset('runtime'), datasize("runtime"))
            return(0, datasize("runtime"))
        }
    }
}
