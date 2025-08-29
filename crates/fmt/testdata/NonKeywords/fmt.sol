struct S {
    uint256 error;
    uint256 layout;
    uint256 at;
    // uint256 transient;
}

function f() {
    uint256 error = 0;
    uint256 layout = 0;
    uint256 at = 0;
    // uint256 transient = 0;

    error = 0;
    // layout = 0;
    at = 0;
    // transient = 0;

    S memory x = S({
        // format
        error: 0,
        layout: 0,
        at: 0
        // transient: 0
    });

    x.error = 0;
    x.layout = 0;
    x.at = 0;
    // x.transient = 0;

    assembly {
        let error := 0
        let layout := 0
        let at := 0
        // let transient := 0

        error := 0
        layout := 0
        at := 0
        // transient := 0
    }
}
