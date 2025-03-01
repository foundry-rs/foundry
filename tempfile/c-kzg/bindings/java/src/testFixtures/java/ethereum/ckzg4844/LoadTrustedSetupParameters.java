package ethereum.ckzg4844;

public class LoadTrustedSetupParameters {

  private final byte[] g1;
  private final long g1Count;
  private final byte[] g2;
  private final long g2Count;

  public LoadTrustedSetupParameters(
      final byte[] g1, final long g1Count, final byte[] g2, final long g2Count) {
    this.g1 = g1;
    this.g1Count = g1Count;
    this.g2 = g2;
    this.g2Count = g2Count;
  }

  public byte[] getG1() {
    return g1;
  }

  public long getG1Count() {
    return g1Count;
  }

  public byte[] getG2() {
    return g2;
  }

  public long getG2Count() {
    return g2Count;
  }
}
