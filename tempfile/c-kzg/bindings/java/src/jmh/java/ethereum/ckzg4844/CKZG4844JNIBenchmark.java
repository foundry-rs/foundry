package ethereum.ckzg4844;

import java.util.concurrent.TimeUnit;
import org.openjdk.jmh.annotations.Benchmark;
import org.openjdk.jmh.annotations.BenchmarkMode;
import org.openjdk.jmh.annotations.Fork;
import org.openjdk.jmh.annotations.Level;
import org.openjdk.jmh.annotations.Measurement;
import org.openjdk.jmh.annotations.Mode;
import org.openjdk.jmh.annotations.OutputTimeUnit;
import org.openjdk.jmh.annotations.Param;
import org.openjdk.jmh.annotations.Scope;
import org.openjdk.jmh.annotations.Setup;
import org.openjdk.jmh.annotations.State;
import org.openjdk.jmh.annotations.TearDown;
import org.openjdk.jmh.annotations.Warmup;

@BenchmarkMode(Mode.AverageTime)
@Fork(value = 1)
@Warmup(iterations = 1, time = 1000, timeUnit = TimeUnit.MILLISECONDS)
@Measurement(iterations = 5, time = 1000, timeUnit = TimeUnit.MILLISECONDS)
@OutputTimeUnit(TimeUnit.MILLISECONDS)
@State(Scope.Benchmark)
public class CKZG4844JNIBenchmark {

  static {
    CKZG4844JNI.loadNativeLibrary();
  }

  @State(Scope.Benchmark)
  public static class BlobToKzgCommitmentState {
    private byte[] blob;

    @Setup(Level.Iteration)
    public void setUp() {
      blob = TestUtils.createRandomBlob();
    }
  }

  @State(Scope.Benchmark)
  public static class ComputeKzgProofState {
    private byte[] blob;
    private byte[] z;

    @Setup(Level.Iteration)
    public void setUp() {
      blob = TestUtils.createRandomBlob();
      z = TestUtils.randomBLSFieldElementBytes();
    }
  }

  @State(Scope.Benchmark)
  public static class ComputeBlobKzgProofState {
    private byte[] blob;
    private byte[] commitment;

    @Setup(Level.Iteration)
    public void setUp() {
      blob = TestUtils.createRandomBlob();
      commitment = TestUtils.createRandomCommitment();
    }
  }

  @State(Scope.Benchmark)
  public static class VerifyKzgProofState {
    private byte[] commitment;
    private byte[] z;
    private byte[] y;
    private byte[] proof;

    @Setup(Level.Iteration)
    public void setUp() {
      commitment = TestUtils.createRandomCommitments(1);
      z = TestUtils.randomBLSFieldElementBytes();
      y = TestUtils.randomBLSFieldElementBytes();
      proof = TestUtils.createRandomProofs(1);
    }
  }

  @State(Scope.Benchmark)
  public static class VerifyBlobKzgProofState {
    private byte[] blob;
    private byte[] commitment;
    private byte[] proof;

    @Setup(Level.Iteration)
    public void setUp() {
      blob = TestUtils.createRandomBlobs(1);
      commitment = TestUtils.createRandomCommitments(1);
      proof = TestUtils.createRandomProofs(1);
    }
  }

  @State(Scope.Benchmark)
  public static class VerifyBlobKzgProofBatchState {
    @Param({"1", "4", "8", "16", "32", "64"})
    private int count;

    private byte[] blobs;
    private byte[] commitments;
    private byte[] proofs;

    @Setup(Level.Iteration)
    public void setUp() {
      blobs = TestUtils.createRandomBlobs(count);
      commitments = TestUtils.createRandomCommitments(count);
      proofs = TestUtils.createRandomProofs(count);
    }
  }

  @Setup
  public void setUp() {
    CKZG4844JNI.loadTrustedSetup("../../src/trusted_setup.txt");
  }

  @TearDown
  public void tearDown() {
    CKZG4844JNI.freeTrustedSetup();
  }

  @Benchmark
  public byte[] blobToKzgCommitment(final BlobToKzgCommitmentState state) {
    return CKZG4844JNI.blobToKzgCommitment(state.blob);
  }

  @Benchmark
  public ProofAndY computeKzgProof(final ComputeKzgProofState state) {
    return CKZG4844JNI.computeKzgProof(state.blob, state.z);
  }

  @Benchmark
  public byte[] computeBlobKzgProof(final ComputeBlobKzgProofState state) {
    return CKZG4844JNI.computeBlobKzgProof(state.blob, state.commitment);
  }

  @Benchmark
  public boolean verifyKzgProof(final VerifyKzgProofState state) {
    return CKZG4844JNI.verifyKzgProof(state.commitment, state.z, state.y, state.proof);
  }

  @Benchmark
  public boolean verifyBlobKzgProof(final VerifyBlobKzgProofState state) {
    return CKZG4844JNI.verifyBlobKzgProof(state.blob, state.commitment, state.proof);
  }

  @Benchmark
  public boolean verifyBlobKzgProofBatch(final VerifyBlobKzgProofBatchState state) {
    return CKZG4844JNI.verifyBlobKzgProofBatch(
        state.blobs, state.commitments, state.proofs, state.count);
  }
}
