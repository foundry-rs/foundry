package ethereum.ckzg4844;

import static ethereum.ckzg4844.CKZGException.CKZGError.C_KZG_BADARGS;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import ethereum.ckzg4844.test_formats.*;
import java.util.stream.IntStream;
import java.util.stream.Stream;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.EnumSource;
import org.junit.jupiter.params.provider.MethodSource;

public class CKZG4844JNITest {
  private enum TrustedSetupSource {
    FILE,
    PARAMETERS,
    RESOURCE
  }

  private static final String TRUSTED_SETUP_FILE = "../../src/trusted_setup.txt";
  private static final String TRUSTED_SETUP_RESOURCE = "/trusted-setups/trusted_setup.txt";
  private static final String OLD_TRUSTED_SETUP_FILE =
      "./src/testFixtures/resources/trusted-setups/trusted_setup_old.txt";

  static {
    CKZG4844JNI.loadNativeLibrary();
  }

  @ParameterizedTest
  @MethodSource("getBlobToKzgCommitmentTests")
  public void blobToKzgCommitmentTests(final BlobToKzgCommitmentTest test) {
    try {
      byte[] commitment = CKZG4844JNI.blobToKzgCommitment(test.getInput().getBlob());
      assertArrayEquals(test.getOutput(), commitment);
    } catch (CKZGException ex) {
      assertNull(test.getOutput());
    }
  }

  @ParameterizedTest
  @MethodSource("getComputeKzgProofTests")
  public void computeKzgProofTests(final ComputeKzgProofTest test) {
    try {
      ProofAndY proofAndY =
          CKZG4844JNI.computeKzgProof(test.getInput().getBlob(), test.getInput().getZ());
      assertArrayEquals(test.getOutput().getProof(), proofAndY.getProof());
      assertArrayEquals(test.getOutput().getY(), proofAndY.getY());
    } catch (CKZGException ex) {
      assertNull(test.getOutput());
    }
  }

  @ParameterizedTest
  @MethodSource("getComputeBlobKzgProofTests")
  public void computeBlobKzgProofTests(final ComputeBlobKzgProofTest test) {
    try {
      byte[] proof =
          CKZG4844JNI.computeBlobKzgProof(
              test.getInput().getBlob(), test.getInput().getCommitment());
      assertArrayEquals(test.getOutput(), proof);
    } catch (CKZGException ex) {
      assertNull(test.getOutput());
    }
  }

  @ParameterizedTest
  @MethodSource("getVerifyKzgProofTests")
  public void verifyKzgProofTests(final VerifyKzgProofTest test) {
    try {
      boolean valid =
          CKZG4844JNI.verifyKzgProof(
              test.getInput().getCommitment(),
              test.getInput().getZ(),
              test.getInput().getY(),
              test.getInput().getProof());
      assertEquals(test.getOutput(), valid);
    } catch (CKZGException ex) {
      assertNull(test.getOutput());
    }
  }

  @ParameterizedTest
  @MethodSource("getVerifyBlobKzgProofTests")
  public void verifyBlobKzgProofTests(final VerifyBlobKzgProofTest test) {
    try {
      boolean valid =
          CKZG4844JNI.verifyBlobKzgProof(
              test.getInput().getBlob(),
              test.getInput().getCommitment(),
              test.getInput().getProof());
      assertEquals(test.getOutput(), valid);
    } catch (CKZGException ex) {
      assertNull(test.getOutput());
    }
  }

  @ParameterizedTest
  @MethodSource("getVerifyBlobKzgProofBatchTests")
  public void verifyBlobKzgProofBatchTests(final VerifyBlobKzgProofBatchTest test) {
    try {
      int count = test.getInput().getBlobs().length / CKZG4844JNI.BYTES_PER_BLOB;
      boolean valid =
          CKZG4844JNI.verifyBlobKzgProofBatch(
              test.getInput().getBlobs(),
              test.getInput().getCommitments(),
              test.getInput().getProofs(),
              count);
      assertEquals(test.getOutput(), valid);
    } catch (CKZGException ex) {
      assertNull(test.getOutput());
    }
  }

  @ParameterizedTest
  @EnumSource(TrustedSetupSource.class)
  public void testVerifyBlobKzgProofBatch(final TrustedSetupSource trustedSetupSource) {
    loadTrustedSetup(trustedSetupSource);
    final int count = 3;
    final byte[][] blobsArray = new byte[count][];
    final byte[][] commitmentsArray = new byte[count][];
    final byte[][] proofsArray = new byte[count][];
    IntStream.range(0, count)
        .forEach(
            i -> {
              blobsArray[i] = TestUtils.createRandomBlob();
              commitmentsArray[i] = CKZG4844JNI.blobToKzgCommitment(blobsArray[i]);
              proofsArray[i] = CKZG4844JNI.computeBlobKzgProof(blobsArray[i], commitmentsArray[i]);
            });
    final byte[] blobs = TestUtils.flatten(blobsArray);
    final byte[] commitments = TestUtils.flatten(commitmentsArray);
    final byte[] proofs = TestUtils.flatten(proofsArray);

    assertTrue(CKZG4844JNI.verifyBlobKzgProofBatch(blobs, commitments, proofs, count));

    final byte[] fakeBlobs = TestUtils.createRandomBlobs(count);
    assertFalse(CKZG4844JNI.verifyBlobKzgProofBatch(fakeBlobs, commitments, proofs, count));
    final byte[] fakeCommitments = TestUtils.createRandomCommitments(count);
    assertFalse(CKZG4844JNI.verifyBlobKzgProofBatch(blobs, fakeCommitments, proofs, count));
    final byte[] fakeProofs = TestUtils.createRandomProofs(count);
    assertFalse(CKZG4844JNI.verifyBlobKzgProofBatch(blobs, commitments, fakeProofs, count));

    CKZG4844JNI.freeTrustedSetup();
  }

  @Test
  public void checkComputeKzgProof() {
    loadTrustedSetup();
    final byte[] blob = TestUtils.createRandomBlob();
    final byte[] z_bytes = TestUtils.randomBLSFieldElementBytes();
    final ProofAndY proofAndY = CKZG4844JNI.computeKzgProof(blob, z_bytes);
    assertEquals(CKZG4844JNI.BYTES_PER_PROOF, proofAndY.getProof().length);
    assertEquals(CKZG4844JNI.BYTES_PER_FIELD_ELEMENT, proofAndY.getY().length);
    CKZG4844JNI.freeTrustedSetup();
  }

  @Test
  public void checkComputeBlobKzgProof() {
    loadTrustedSetup();
    final byte[] blob = TestUtils.createRandomBlob();
    final byte[] commitment = TestUtils.createRandomCommitment();
    final byte[] proof = CKZG4844JNI.computeBlobKzgProof(blob, commitment);
    assertEquals(CKZG4844JNI.BYTES_PER_PROOF, proof.length);
    CKZG4844JNI.freeTrustedSetup();
  }

  @Test
  public void checkCustomExceptionIsThrownAsExpected() {

    loadTrustedSetup();

    final byte[] blob = TestUtils.createNonCanonicalBlob();

    final CKZGException exception =
        assertThrows(CKZGException.class, () -> CKZG4844JNI.blobToKzgCommitment(blob));

    assertEquals(C_KZG_BADARGS, exception.getError());
    assertEquals("There was an error in blobToKzgCommitment.", exception.getErrorMessage());

    CKZG4844JNI.freeTrustedSetup();
  }

  @Test
  public void passingDifferentLengthForCommitmentsThrowsAnException() {
    loadTrustedSetup();

    final int count = 2;
    final byte[] blobs = TestUtils.createRandomBlobs(count);
    final byte[] proofs = TestUtils.createRandomProofs(count);
    // different length for commitments
    final byte[] commitments = TestUtils.createRandomCommitments(3);

    final CKZGException exception =
        assertThrows(
            CKZGException.class,
            () -> CKZG4844JNI.verifyBlobKzgProofBatch(blobs, commitments, proofs, count));
    assertEquals(C_KZG_BADARGS, exception.getError());
    assertEquals(
        "Invalid commitments size. Expected 96 bytes but got 144.", exception.getErrorMessage());

    CKZG4844JNI.freeTrustedSetup();
  }

  @Test
  public void passingInvalidLengthForBlobsThrowsAnException() {

    loadTrustedSetup();

    CKZGException exception =
        assertThrows(CKZGException.class, () -> CKZG4844JNI.blobToKzgCommitment(new byte[0]));

    assertEquals(C_KZG_BADARGS, exception.getError());
    assertEquals(
        String.format(
            "Invalid blob size. Expected %d bytes but got 0.", CKZG4844JNI.BYTES_PER_BLOB),
        exception.getErrorMessage());

    exception =
        assertThrows(
            CKZGException.class,
            () -> CKZG4844JNI.computeBlobKzgProof(new byte[123], new byte[32]));

    assertEquals(C_KZG_BADARGS, exception.getError());
    assertEquals(
        String.format(
            "Invalid blob size. Expected %d bytes but got 123.", CKZG4844JNI.BYTES_PER_BLOB),
        exception.getErrorMessage());

    exception =
        assertThrows(
            CKZGException.class,
            () ->
                CKZG4844JNI.computeBlobKzgProof(
                    new byte[CKZG4844JNI.BYTES_PER_BLOB], new byte[49]));

    assertEquals(C_KZG_BADARGS, exception.getError());
    assertEquals(
        "Invalid commitment size. Expected 48 bytes but got 49.", exception.getErrorMessage());

    exception =
        assertThrows(
            CKZGException.class,
            () ->
                CKZG4844JNI.verifyBlobKzgProofBatch(
                    new byte[42],
                    TestUtils.createRandomCommitments(2),
                    TestUtils.createRandomProofs(2),
                    2));

    assertEquals(C_KZG_BADARGS, exception.getError());
    assertEquals(
        String.format(
            "Invalid blobs size. Expected %d bytes but got 42.", CKZG4844JNI.BYTES_PER_BLOB * 2),
        exception.getErrorMessage());

    CKZG4844JNI.freeTrustedSetup();
  }

  @Test
  public void throwsIfMethodIsUsedWithoutLoadingTrustedSetup() {

    final RuntimeException exception =
        assertThrows(
            RuntimeException.class,
            () -> CKZG4844JNI.blobToKzgCommitment(TestUtils.createRandomBlob()));

    assertExceptionIsTrustedSetupIsNotLoaded(exception);
  }

  @Test
  public void throwsIfSetupIsLoadedTwice() {

    loadTrustedSetup();

    final RuntimeException exception =
        assertThrows(RuntimeException.class, CKZG4844JNITest::loadTrustedSetup);

    assertEquals(
        "Trusted Setup is already loaded. Free it before loading a new one.",
        exception.getMessage());

    CKZG4844JNI.freeTrustedSetup();
  }

  @Test
  public void throwsIfTryToFreeTrustedSetupWithoutLoadingIt() {

    final RuntimeException exception =
        assertThrows(RuntimeException.class, CKZG4844JNI::freeTrustedSetup);

    assertExceptionIsTrustedSetupIsNotLoaded(exception);
  }

  @Test
  public void shouldThrowExceptionIfTrustedSetupIsNotInLagrangeForm() {
    CKZGException exception =
        assertThrows(
            CKZGException.class, () -> CKZG4844JNI.loadTrustedSetup(OLD_TRUSTED_SETUP_FILE));

    assertEquals(C_KZG_BADARGS, exception.getError());
  }

  @Test
  public void shouldThrowExceptionOnIncorrectTrustedSetupParameters() {
    final LoadTrustedSetupParameters parameters =
        TestUtils.createLoadTrustedSetupParameters(TRUSTED_SETUP_FILE);

    // wrong g1Count
    CKZGException exception =
        assertThrows(
            CKZGException.class,
            () ->
                CKZG4844JNI.loadTrustedSetup(
                    parameters.getG1(),
                    parameters.getG1Count() + 1,
                    parameters.getG2(),
                    parameters.getG2Count()));
    assertEquals(C_KZG_BADARGS, exception.getError());
    assertTrue(exception.getErrorMessage().contains("Invalid g1 size."));

    // wrong g2Count
    exception =
        assertThrows(
            CKZGException.class,
            () ->
                CKZG4844JNI.loadTrustedSetup(
                    parameters.getG1(),
                    parameters.getG1Count(),
                    parameters.getG2(),
                    parameters.getG2Count() + 1));
    assertEquals(C_KZG_BADARGS, exception.getError());
    assertTrue(exception.getErrorMessage().contains("Invalid g2 size."));
  }

  private void assertExceptionIsTrustedSetupIsNotLoaded(final RuntimeException exception) {
    assertEquals("Trusted Setup is not loaded.", exception.getMessage());
  }

  private static void loadTrustedSetup(final TrustedSetupSource trustedSetupSource) {
    switch (trustedSetupSource) {
      case FILE:
        loadTrustedSetup();
        break;
      case PARAMETERS:
        loadTrustedSetupFromParameters();
        break;
      case RESOURCE:
        loadTrustedSetupFromResource();
        break;
    }
  }

  private static void loadTrustedSetup() {
    CKZG4844JNI.loadTrustedSetup(TRUSTED_SETUP_FILE);
  }

  private static void loadTrustedSetupFromParameters() {
    final LoadTrustedSetupParameters parameters =
        TestUtils.createLoadTrustedSetupParameters(TRUSTED_SETUP_FILE);
    CKZG4844JNI.loadTrustedSetup(
        parameters.getG1(), parameters.getG1Count(), parameters.getG2(), parameters.getG2Count());
  }

  public static void loadTrustedSetupFromResource() {
    CKZG4844JNI.loadTrustedSetupFromResource(TRUSTED_SETUP_RESOURCE, CKZG4844JNITest.class);
  }

  private static Stream<BlobToKzgCommitmentTest> getBlobToKzgCommitmentTests() {
    loadTrustedSetup();
    return TestUtils.getBlobToKzgCommitmentTests().stream().onClose(CKZG4844JNI::freeTrustedSetup);
  }

  private static Stream<ComputeKzgProofTest> getComputeKzgProofTests() {
    loadTrustedSetup();
    return TestUtils.getComputeKzgProofTests().stream().onClose(CKZG4844JNI::freeTrustedSetup);
  }

  private static Stream<ComputeBlobKzgProofTest> getComputeBlobKzgProofTests() {
    loadTrustedSetup();
    return TestUtils.getComputeBlobKzgProofTests().stream().onClose(CKZG4844JNI::freeTrustedSetup);
  }

  private static Stream<VerifyKzgProofTest> getVerifyKzgProofTests() {
    loadTrustedSetup();
    return TestUtils.getVerifyKzgProofTests().stream().onClose(CKZG4844JNI::freeTrustedSetup);
  }

  private static Stream<VerifyBlobKzgProofTest> getVerifyBlobKzgProofTests() {
    loadTrustedSetup();
    return TestUtils.getVerifyBlobKzgProofTests().stream().onClose(CKZG4844JNI::freeTrustedSetup);
  }

  private static Stream<VerifyBlobKzgProofBatchTest> getVerifyBlobKzgProofBatchTests() {
    loadTrustedSetup();
    return TestUtils.getVerifyBlobKzgProofBatchTests().stream()
        .onClose(CKZG4844JNI::freeTrustedSetup);
  }
}
