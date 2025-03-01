mod helpers;

use helpers::ignore_tests::*;

#[tokio::test]
async fn globals() {
	let filter = filt(
		"tree",
		&[
			file("global/first").applies_globally(),
			file("global/second").applies_globally(),
		],
	)
	.await;

	// Both ignores should be loaded as global
	filter.agnostic_fail("/apples");
	filter.agnostic_fail("/oranges");

	// Sanity check
	filter.agnostic_pass("/kiwi");
}

#[tokio::test]
async fn tree() {
	let filter = filt("tree", &[file("tree/base"), file("tree/branch/inner")]).await;

	// "oranges" is not ignored at any level
	filter.agnostic_pass("tree/oranges");
	filter.agnostic_pass("tree/branch/oranges");
	filter.agnostic_pass("tree/branch/inner/oranges");
	filter.agnostic_pass("tree/other/oranges");

	// "apples" should only be ignored at the root
	filter.agnostic_fail("tree/apples");
	filter.agnostic_pass("tree/branch/apples");
	filter.agnostic_pass("tree/branch/inner/apples");
	filter.agnostic_pass("tree/other/apples");

	// "carrots" should be ignored at any level
	filter.agnostic_fail("tree/carrots");
	filter.agnostic_fail("tree/branch/carrots");
	filter.agnostic_fail("tree/branch/inner/carrots");
	filter.agnostic_fail("tree/other/carrots");

	// "pineapples/grapes" should only be ignored at the root
	filter.agnostic_fail("tree/pineapples/grapes");
	filter.agnostic_pass("tree/branch/pineapples/grapes");
	filter.agnostic_pass("tree/branch/inner/pineapples/grapes");
	filter.agnostic_pass("tree/other/pineapples/grapes");

	// "cauliflowers" should only be ignored at the root of "branch/"
	filter.agnostic_pass("tree/cauliflowers");
	filter.agnostic_fail("tree/branch/cauliflowers");
	filter.agnostic_pass("tree/branch/inner/cauliflowers");
	filter.agnostic_pass("tree/other/cauliflowers");

	// "artichokes" should be ignored anywhere inside of "branch/"
	filter.agnostic_pass("tree/artichokes");
	filter.agnostic_fail("tree/branch/artichokes");
	filter.agnostic_fail("tree/branch/inner/artichokes");
	filter.agnostic_pass("tree/other/artichokes");

	// "bananas/pears" should only be ignored at the root of "branch/"
	filter.agnostic_pass("tree/bananas/pears");
	filter.agnostic_fail("tree/branch/bananas/pears");
	filter.agnostic_pass("tree/branch/inner/bananas/pears");
	filter.agnostic_pass("tree/other/bananas/pears");
}
