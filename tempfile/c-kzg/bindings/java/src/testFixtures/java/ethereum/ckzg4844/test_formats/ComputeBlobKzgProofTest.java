package ethereum.ckzg4844.test_formats;

import org.apache.tuweni.bytes.Bytes;

public class ComputeBlobKzgProofTest {
  public static class Input {
    private String blob;
    private String commitment;

    public byte[] getBlob() {
      return Bytes.fromHexString(blob).toArrayUnsafe();
    }

    public byte[] getCommitment() {
      return Bytes.fromHexString(commitment).toArray();
    }
  }

  private Input input;
  private String output;

  public Input getInput() {
    return input;
  }

  public byte[] getOutput() {
    if (output == null) {
      return null;
    }
    return Bytes.fromHexString(output).toArray();
  }
}
