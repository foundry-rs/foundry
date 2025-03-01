package ethereum.ckzg4844.test_formats;

import org.apache.tuweni.bytes.Bytes;

public class VerifyKzgProofTest {
  public static class Input {
    private String commitment;
    private String z;
    private String y;
    private String proof;

    public byte[] getCommitment() {
      return Bytes.fromHexString(commitment).toArray();
    }

    public byte[] getZ() {
      return Bytes.fromHexString(z).toArray();
    }

    public byte[] getY() {
      return Bytes.fromHexString(y).toArray();
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
