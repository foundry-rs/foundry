package ethereum.ckzg4844.test_formats;

import org.apache.tuweni.bytes.Bytes;

public class VerifyBlobKzgProofTest {
  public static class Input {
    private String blob;
    private String commitment;
    private String proof;

    public byte[] getBlob() {
      return Bytes.fromHexString(blob).toArrayUnsafe();
    }

    public byte[] getCommitment() {
      return Bytes.fromHexString(commitment).toArray();
    }

    public byte[] getProof() {
      return Bytes.fromHexString(proof).toArray();
    }
  }

  private Input input;
  private Boolean output;

  public Input getInput() {
    return input;
  }

  public Boolean getOutput() {
    return output;
  }
}
