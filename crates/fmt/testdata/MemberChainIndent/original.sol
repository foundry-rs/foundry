library L {
  function f() internal {
    require(
      vars.receiver.executeOperation(
        params.assets,
        params.amounts,
        vars.totalPremiums,
        params.user,
        params.params
      ),
      Errors.InvalidFlashloanExecutorReturn()
    );
  }
}
