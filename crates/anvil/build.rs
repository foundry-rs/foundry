fn main() {
    vergen::EmitBuilder::builder().build_timestamp().git_sha(true).emit().unwrap();
}
