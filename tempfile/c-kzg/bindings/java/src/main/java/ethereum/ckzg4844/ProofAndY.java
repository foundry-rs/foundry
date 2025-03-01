package ethereum.ckzg4844;

public class ProofAndY {
  private final byte[] proof;
  private final byte[] y;

  public ProofAndY(byte[] proof, byte[] y) {
    this.proof = proof;
    this.y = y;
  }

  public byte[] getProof() {
    return proof;
  }

  public byte[] getY() {
    return y;
  }

  public static ProofAndY of(byte[] proof, byte[] y) {
    return new ProofAndY(proof, y);
  }
}
