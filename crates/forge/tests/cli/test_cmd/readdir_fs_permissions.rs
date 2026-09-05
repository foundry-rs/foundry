// CLI integration tests for `vm.readDir`'s `fs_permissions` boundary.
//
// `read_dir` (crates/cheatcodes/src/fs.rs) only checked the root path passed to `readDir`
// against `fs_permissions`. With `followLinks: true`, `WalkDir` follows a symlink inside a
// permitted directory out to wherever it points, and every entry beyond it was returned
// completely unchecked - a permitted directory containing a stray symlink could be used to
// enumerate file/directory *names* anywhere else on disk that the walk could reach.
// `vm.readFile` on the identical escaped path was (and still is) correctly rejected; only
// `readDir` skipped the equivalent check for entries found while walking.

use foundry_config::fs_permissions::PathPermission;
use foundry_test_utils::forgetest_init;
use std::fs;

#[cfg(unix)]
forgetest_init!(readdir_does_not_follow_symlink_out_of_fs_permissions_boundary, |prj, cmd| {
    // A directory genuinely outside the project entirely, not just outside the permitted
    // subpath - anything reachable through the escaping symlink should be unreachable via
    // `readDir` regardless of how deep the walk follows it.
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), "leaked").unwrap();
    let outside_sub = outside.path().join("sub");
    fs::create_dir(&outside_sub).unwrap();
    fs::write(outside_sub.join("nested.txt"), "leaked-nested").unwrap();

    let allowed = prj.root().join("allowed");
    fs::create_dir(&allowed).unwrap();
    fs::write(allowed.join("normal.txt"), "fine").unwrap();
    std::os::unix::fs::symlink(outside.path(), allowed.join("escape")).unwrap();

    prj.update_config(|config| {
        config.fs_permissions.add(PathPermission::read(prj.root().join("allowed")));
    });

    prj.add_raw_test(
        "ReadDirSymlinkEscape.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface Vm {
    struct DirEntry {
        string errorMessage;
        string path;
        uint64 depth;
        bool isDir;
        bool isSymlink;
    }

    function readDir(string calldata path, uint64 maxDepth, bool followLinks)
        external
        view
        returns (DirEntry[] memory entries);
    function readFile(string calldata path) external view returns (string memory data);
    function _expectCheatcodeRevert() external;
}

contract ReadDirSymlinkEscapeTest {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function _hasPathEndingWith(Vm.DirEntry[] memory entries, string memory suffix)
        internal
        pure
        returns (bool)
    {
        bytes memory suffixBytes = bytes(suffix);
        for (uint256 i = 0; i < entries.length; i++) {
            bytes memory p = bytes(entries[i].path);
            if (p.length < suffixBytes.length) continue;
            bool matches = true;
            for (uint256 j = 0; j < suffixBytes.length; j++) {
                if (p[p.length - suffixBytes.length + j] != suffixBytes[j]) {
                    matches = false;
                    break;
                }
            }
            if (matches) return true;
        }
        return false;
    }

    // followLinks: true must not surface anything beyond the escaping symlink, at any depth,
    // while still listing the normal in-bounds entries and the symlink's own (in-bounds) name.
    function test_followLinksTrue_excludesEscapedEntries() public {
        Vm.DirEntry[] memory entries = vm.readDir("allowed", 10, true);

        require(!_hasPathEndingWith(entries, "secret.txt"), "leaked top-level escaped file");
        require(!_hasPathEndingWith(entries, "nested.txt"), "leaked nested escaped file");
        require(!_hasPathEndingWith(entries, "/sub"), "leaked escaped subdirectory");

        require(_hasPathEndingWith(entries, "normal.txt"), "normal in-bounds entry missing");
        require(_hasPathEndingWith(entries, "escape"), "symlink's own name should still list");
        require(entries.length == 2, "expected exactly the two in-bounds entries");
    }

    // followLinks: false must be completely unaffected: the symlink itself is still listed
    // (it lives inside the permitted directory), it's just never descended into.
    function test_followLinksFalse_unchanged() public {
        Vm.DirEntry[] memory entries = vm.readDir("allowed", 10, false);

        require(!_hasPathEndingWith(entries, "secret.txt"));
        require(_hasPathEndingWith(entries, "normal.txt"));
        require(_hasPathEndingWith(entries, "escape"));
        require(entries.length == 2);
    }

    // Control: readFile on the exact same escaped path was already correctly rejected before
    // this fix and must remain so - readDir's leak was in listing names, never in granting
    // content access.
    function test_readFile_sameEscapedPath_stillRejected() public {
        vm._expectCheatcodeRevert();
        vm.readFile("allowed/escape/secret.txt");
    }
}
"#,
    );

    cmd.args([
        "test",
        "--mt",
        "test_(followLinksTrue_excludesEscapedEntries|followLinksFalse_unchanged|readFile_sameEscapedPath_stillRejected)",
    ])
    .assert_success();
});

// A symlink that escapes the sandbox and then, further down its own target tree, points back
// to somewhere legitimately in-bounds must not leak any of the outside path components it took
// to get there. Canonicalizing only an entry's immediate parent is not enough to catch this:
// the final destination can resolve as "allowed" even though the walk had to step outside the
// sandbox to reach it, so the fix must stop descending at the first escaping step rather than
// judging each entry purely by where it ultimately, fully-resolved, ends up.
#[cfg(unix)]
forgetest_init!(readdir_does_not_leak_names_when_escape_reenters_boundary, |prj, cmd| {
    let allowed = prj.root().join("allowed");
    fs::create_dir(&allowed).unwrap();
    let local = allowed.join("local");
    fs::create_dir(&local).unwrap();
    fs::write(local.join("visible.txt"), "fine").unwrap();

    let outside = tempfile::tempdir().unwrap();
    let hidden_dir = outside.path().join("hidden_dir");
    fs::create_dir(&hidden_dir).unwrap();
    // Points back inside the sandbox - but reaching it at all requires having already stepped
    // through `outside/`, which is not permitted.
    std::os::unix::fs::symlink(&local, hidden_dir.join("reentry")).unwrap();

    std::os::unix::fs::symlink(outside.path(), allowed.join("escape")).unwrap();

    prj.update_config(|config| {
        config.fs_permissions.add(PathPermission::read(prj.root().join("allowed")));
    });

    prj.add_raw_test(
        "ReadDirEscapeReentry.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface Vm {
    struct DirEntry {
        string errorMessage;
        string path;
        uint64 depth;
        bool isDir;
        bool isSymlink;
    }

    function readDir(string calldata path, uint64 maxDepth, bool followLinks)
        external
        view
        returns (DirEntry[] memory entries);
}

contract ReadDirEscapeReentryTest {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function _hasPathEndingWith(Vm.DirEntry[] memory entries, string memory suffix)
        internal
        pure
        returns (bool)
    {
        bytes memory suffixBytes = bytes(suffix);
        for (uint256 i = 0; i < entries.length; i++) {
            bytes memory p = bytes(entries[i].path);
            if (p.length < suffixBytes.length) continue;
            bool matches = true;
            for (uint256 j = 0; j < suffixBytes.length; j++) {
                if (p[p.length - suffixBytes.length + j] != suffixBytes[j]) {
                    matches = false;
                    break;
                }
            }
            if (matches) return true;
        }
        return false;
    }

    function test_escapeThenReenter_doesNotLeakIntermediateNames() public {
        Vm.DirEntry[] memory entries = vm.readDir("allowed", 10, true);

        require(!_hasPathEndingWith(entries, "hidden_dir"), "leaked outside directory name");
        require(!_hasPathEndingWith(entries, "reentry"), "leaked outside symlink name");
        require(
            !_hasPathEndingWith(entries, "escape/hidden_dir/reentry/visible.txt"),
            "leaked file reached only via the escaped chain"
        );
        // Reached directly through the in-bounds `allowed/local`, unrelated to the escape - must
        // still be listed once.
        require(_hasPathEndingWith(entries, "local/visible.txt"), "direct in-bounds entry missing");
    }
}
"#,
    );

    cmd.args(["test", "--mt", "test_escapeThenReenter_doesNotLeakIntermediateNames"])
        .assert_success();
});

// A root that's genuinely authorized (granted directly, not merely inheriting from a permitted
// ancestor) must still surface its own read error - e.g. because it doesn't exist - rather than
// silently returning an empty list. The root is exempt from the parent-boundary check (it was
// already validated by `ensure_path_allowed` before the walk starts), and that exemption must
// extend to the root's *error* entry, not just a successful one.
#[cfg(unix)]
forgetest_init!(readdir_root_error_preserved_even_if_parent_unpermitted, |prj, cmd| {
    // Permission is granted directly on `missing_root` itself; its parent (`prj.root()`) is not
    // separately permitted, so a naive parent-based check on the root's own error would drop it.
    let missing_root = prj.root().join("missing_root");

    prj.update_config(|config| {
        config.fs_permissions.add(PathPermission::read(missing_root.clone()));
    });

    prj.add_raw_test(
        "ReadDirMissingRoot.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface Vm {
    struct DirEntry {
        string errorMessage;
        string path;
        uint64 depth;
        bool isDir;
        bool isSymlink;
    }

    function readDir(string calldata path, uint64 maxDepth, bool followLinks)
        external
        view
        returns (DirEntry[] memory entries);
}

contract ReadDirMissingRootTest {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function test_missingRoot_stillReportsItsOwnError() public {
        Vm.DirEntry[] memory entries = vm.readDir("missing_root", 1, true);
        require(entries.length == 1, "expected the root's own error entry, got an empty list");
        require(bytes(entries[0].errorMessage).length > 0, "root error message was dropped");
    }
}
"#,
    );

    cmd.args(["test", "--mt", "test_missingRoot_stillReportsItsOwnError"]).assert_success();
});

// A permitted directory containing a symlink to *another* permitted directory must still list
// everything reached through it with `followLinks: true` - the fix must stop an escaping walk,
// not a legitimate one.
#[cfg(unix)]
forgetest_init!(readdir_still_follows_symlink_to_another_permitted_dir, |prj, cmd| {
    let allowed = prj.root().join("allowed");
    fs::create_dir(&allowed).unwrap();
    let allowed_more = prj.root().join("allowed_more");
    fs::create_dir(&allowed_more).unwrap();
    fs::write(allowed_more.join("deep.txt"), "fine").unwrap();
    std::os::unix::fs::symlink(&allowed_more, allowed.join("link_to_more")).unwrap();

    prj.update_config(|config| {
        config.fs_permissions.add(PathPermission::read(prj.root().join("allowed")));
        config.fs_permissions.add(PathPermission::read(prj.root().join("allowed_more")));
    });

    prj.add_raw_test(
        "ReadDirLegitimateSymlink.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface Vm {
    struct DirEntry {
        string errorMessage;
        string path;
        uint64 depth;
        bool isDir;
        bool isSymlink;
    }

    function readDir(string calldata path, uint64 maxDepth, bool followLinks)
        external
        view
        returns (DirEntry[] memory entries);
}

contract ReadDirLegitimateSymlinkTest {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function _hasPathEndingWith(Vm.DirEntry[] memory entries, string memory suffix)
        internal
        pure
        returns (bool)
    {
        bytes memory suffixBytes = bytes(suffix);
        for (uint256 i = 0; i < entries.length; i++) {
            bytes memory p = bytes(entries[i].path);
            if (p.length < suffixBytes.length) continue;
            bool matches = true;
            for (uint256 j = 0; j < suffixBytes.length; j++) {
                if (p[p.length - suffixBytes.length + j] != suffixBytes[j]) {
                    matches = false;
                    break;
                }
            }
            if (matches) return true;
        }
        return false;
    }

    function test_legitimateSymlinkTarget_stillListed() public {
        Vm.DirEntry[] memory entries = vm.readDir("allowed", 10, true);
        require(_hasPathEndingWith(entries, "link_to_more"), "symlink's own name missing");
        require(_hasPathEndingWith(entries, "link_to_more/deep.txt"), "legitimate symlink target not followed");
    }
}
"#,
    );

    cmd.args(["test", "--mt", "test_legitimateSymlinkTarget_stillListed"]).assert_success();
});

// A permitted directory with no escaping symlink at all must list identically to before this
// fix - the parent-boundary check must never affect an ordinary, fully in-bounds walk.
#[cfg(unix)]
forgetest_init!(readdir_unaffected_when_no_symlink_present, |prj, cmd| {
    let allowed = prj.root().join("allowed_plain");
    fs::create_dir(&allowed).unwrap();
    fs::write(allowed.join("a.txt"), "a").unwrap();
    let nested = allowed.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("b.txt"), "b").unwrap();

    prj.update_config(|config| {
        config.fs_permissions.add(PathPermission::read(prj.root().join("allowed_plain")));
    });

    prj.add_raw_test(
        "ReadDirNoSymlink.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface Vm {
    struct DirEntry {
        string errorMessage;
        string path;
        uint64 depth;
        bool isDir;
        bool isSymlink;
    }

    function readDir(string calldata path, uint64 maxDepth, bool followLinks)
        external
        view
        returns (DirEntry[] memory entries);
}

contract ReadDirNoSymlinkTest {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function test_ordinaryWalk_listsEverything() public {
        Vm.DirEntry[] memory entries = vm.readDir("allowed_plain", 10, true);
        require(entries.length == 3, "a.txt, nested/, nested/b.txt");
    }
}
"#,
    );

    cmd.args(["test", "--mt", "test_ordinaryWalk_listsEverything"]).assert_success();
});
