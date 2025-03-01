package ethereum.ckzg4844.test_formats;

import ethereum.ckzg4844.TestUtils;
import java.util.List;
import java.util.stream.Collectors;
import org.apache.tuweni.bytes.Bytes;

public class VerifyBlobKzgProofBatchTest {
  public static class Input {
    private List<String> blobs;
    private List<String> commitments;
    private List<String> proofs;

    public byte[] getBlobs() {
      return TestUtils.flatten(
          blobs.stream()
              .map(Bytes::fromHexString)
              .map(Bytes::toArrayUnsafe)
              .collect(Collectors.toList())
              .toArray(byte[][]::new));
    }

    public byte[] getCommitments() {
      return TestUtils.flatten(
          commitments.stream()
              .map(Bytes::fromHexString)
              .map(Bytes::toArray)
              .collect(Collectors.toList())
              .toArray(byte[][]::new));
    }

    public byte[] getProofs() {
      return TestUtils.flatten(
          proofs.stream()
              .map(Bytes::fromHexString)
              .map(Bytes::toArray)
              .collect(Collectors.toList())
              .toArray(byte[][]::new));
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
