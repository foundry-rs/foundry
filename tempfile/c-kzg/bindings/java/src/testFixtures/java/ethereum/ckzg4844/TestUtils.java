package ethereum.ckzg4844;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.dataformat.yaml.YAMLFactory;
import ethereum.ckzg4844.test_formats.*;
import java.io.BufferedReader;
import java.io.FileReader;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.math.BigInteger;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Random;
import java.util.stream.Collectors;
import java.util.stream.IntStream;
import java.util.stream.Stream;
import org.apache.tuweni.bytes.Bytes;
import org.apache.tuweni.units.bigints.UInt256;

public class TestUtils {

  private static final ObjectMapper OBJECT_MAPPER = new ObjectMapper(new YAMLFactory());

  private static final Random RANDOM = new Random();

  private static final String BLOB_TO_KZG_COMMITMENT_TESTS = "../../tests/blob_to_kzg_commitment/";
  private static final String COMPUTE_KZG_PROOF_TESTS = "../../tests/compute_kzg_proof/";
  private static final String COMPUTE_BLOB_KZG_PROOF_TESTS = "../../tests/compute_blob_kzg_proof/";
  private static final String VERIFY_KZG_PROOF_TESTS = "../../tests/verify_kzg_proof/";
  private static final String VERIFY_BLOB_KZG_PROOF_TESTS = "../../tests/verify_blob_kzg_proof/";
  private static final String VERIFY_BLOB_KZG_PROOF_BATCH_TESTS =
      "../../tests/verify_blob_kzg_proof_batch/";

  public static byte[] flatten(final byte[]... bytes) {
    final int capacity = Arrays.stream(bytes).mapToInt(b -> b.length).sum();
    final ByteBuffer buffer = ByteBuffer.allocate(capacity);
    Arrays.stream(bytes).forEach(buffer::put);
    return buffer.array();
  }

  public static byte[] createRandomBlob() {
    final byte[][] blob =
        IntStream.range(0, CKZG4844JNI.FIELD_ELEMENTS_PER_BLOB)
            .mapToObj(__ -> randomBLSFieldElement())
            .map(fieldElement -> fieldElement.toArray(ByteOrder.BIG_ENDIAN))
            .toArray(byte[][]::new);
    return flatten(blob);
  }

  public static byte[] createRandomBlobs(final int count) {
    final byte[][] blobs =
        IntStream.range(0, count).mapToObj(__ -> createRandomBlob()).toArray(byte[][]::new);
    return flatten(blobs);
  }

  public static byte[] createRandomProof() {
    return CKZG4844JNI.computeBlobKzgProof(createRandomBlob(), createRandomCommitment());
  }

  public static byte[] createRandomProofs(final int count) {
    final byte[][] proofs =
        IntStream.range(0, count).mapToObj(__ -> createRandomProof()).toArray(byte[][]::new);
    return flatten(proofs);
  }

  public static byte[] createRandomCommitment() {
    return CKZG4844JNI.blobToKzgCommitment(createRandomBlob());
  }

  public static byte[] createRandomCommitments(final int count) {
    final byte[][] commitments =
        IntStream.range(0, count).mapToObj(__ -> createRandomCommitment()).toArray(byte[][]::new);
    return flatten(commitments);
  }

  public static byte[] createNonCanonicalBlob() {
    final byte[][] blob =
        IntStream.range(0, CKZG4844JNI.FIELD_ELEMENTS_PER_BLOB)
            .mapToObj(__ -> UInt256.valueOf(CKZG4844JNI.BLS_MODULUS.add(BigInteger.valueOf(42))))
            .map(greaterThanModulus -> greaterThanModulus.toArray(ByteOrder.BIG_ENDIAN))
            .toArray(byte[][]::new);
    return flatten(blob);
  }

  public static List<BlobToKzgCommitmentTest> getBlobToKzgCommitmentTests() {
    final Stream.Builder<BlobToKzgCommitmentTest> tests = Stream.builder();
    List<String> testFiles = getTestFiles(BLOB_TO_KZG_COMMITMENT_TESTS);
    assert !testFiles.isEmpty();

    try {
      for (String testFile : testFiles) {
        String data = Files.readString(Path.of(testFile));
        BlobToKzgCommitmentTest test = OBJECT_MAPPER.readValue(data, BlobToKzgCommitmentTest.class);
        tests.add(test);
      }
    } catch (IOException ex) {
      throw new UncheckedIOException(ex);
    }

    return tests.build().collect(Collectors.toList());
  }

  public static List<ComputeKzgProofTest> getComputeKzgProofTests() {
    final Stream.Builder<ComputeKzgProofTest> tests = Stream.builder();
    List<String> testFiles = getTestFiles(COMPUTE_KZG_PROOF_TESTS);
    assert !testFiles.isEmpty();

    try {
      for (String testFile : testFiles) {
        String jsonData = Files.readString(Path.of(testFile));
        ComputeKzgProofTest test = OBJECT_MAPPER.readValue(jsonData, ComputeKzgProofTest.class);
        tests.add(test);
      }
    } catch (IOException ex) {
      throw new UncheckedIOException(ex);
    }

    return tests.build().collect(Collectors.toList());
  }

  public static List<ComputeBlobKzgProofTest> getComputeBlobKzgProofTests() {
    final Stream.Builder<ComputeBlobKzgProofTest> tests = Stream.builder();
    List<String> testFiles = getTestFiles(COMPUTE_BLOB_KZG_PROOF_TESTS);
    assert !testFiles.isEmpty();

    try {
      for (String testFile : testFiles) {
        String jsonData = Files.readString(Path.of(testFile));
        ComputeBlobKzgProofTest test =
            OBJECT_MAPPER.readValue(jsonData, ComputeBlobKzgProofTest.class);
        tests.add(test);
      }
    } catch (IOException ex) {
      throw new UncheckedIOException(ex);
    }

    return tests.build().collect(Collectors.toList());
  }

  public static List<VerifyKzgProofTest> getVerifyKzgProofTests() {
    final Stream.Builder<VerifyKzgProofTest> tests = Stream.builder();
    List<String> testFiles = getTestFiles(VERIFY_KZG_PROOF_TESTS);
    assert !testFiles.isEmpty();

    try {
      for (String testFile : testFiles) {
        String jsonData = Files.readString(Path.of(testFile));
        VerifyKzgProofTest test = OBJECT_MAPPER.readValue(jsonData, VerifyKzgProofTest.class);
        tests.add(test);
      }
    } catch (IOException ex) {
      throw new UncheckedIOException(ex);
    }

    return tests.build().collect(Collectors.toList());
  }

  public static List<VerifyBlobKzgProofTest> getVerifyBlobKzgProofTests() {
    final Stream.Builder<VerifyBlobKzgProofTest> tests = Stream.builder();
    List<String> testFiles = getTestFiles(VERIFY_BLOB_KZG_PROOF_TESTS);
    assert !testFiles.isEmpty();

    try {
      for (String testFile : testFiles) {
        String jsonData = Files.readString(Path.of(testFile));
        VerifyBlobKzgProofTest test =
            OBJECT_MAPPER.readValue(jsonData, VerifyBlobKzgProofTest.class);
        tests.add(test);
      }
    } catch (IOException ex) {
      throw new UncheckedIOException(ex);
    }

    return tests.build().collect(Collectors.toList());
  }

  public static List<VerifyBlobKzgProofBatchTest> getVerifyBlobKzgProofBatchTests() {
    final Stream.Builder<VerifyBlobKzgProofBatchTest> tests = Stream.builder();
    List<String> testFiles = getTestFiles(VERIFY_BLOB_KZG_PROOF_BATCH_TESTS);
    assert !testFiles.isEmpty();

    try {
      for (String testFile : testFiles) {
        String jsonData = Files.readString(Path.of(testFile));
        VerifyBlobKzgProofBatchTest test =
            OBJECT_MAPPER.readValue(jsonData, VerifyBlobKzgProofBatchTest.class);
        tests.add(test);
      }
    } catch (IOException ex) {
      throw new UncheckedIOException(ex);
    }

    return tests.build().collect(Collectors.toList());
  }

  public static LoadTrustedSetupParameters createLoadTrustedSetupParameters(
      final String trustedSetup) {
    try (final BufferedReader reader = new BufferedReader(new FileReader(trustedSetup))) {
      final int g1Count = Integer.parseInt(reader.readLine());
      final int g2Count = Integer.parseInt(reader.readLine());

      final ByteBuffer g1 = ByteBuffer.allocate(g1Count * CKZG4844JNI.BYTES_PER_G1);
      final ByteBuffer g2 = ByteBuffer.allocate(g2Count * CKZG4844JNI.BYTES_PER_G2);

      for (int i = 0; i < g1Count; i++) {
        g1.put(Bytes.fromHexString(reader.readLine()).toArray());
      }
      for (int i = 0; i < g2Count; i++) {
        g2.put(Bytes.fromHexString(reader.readLine()).toArray());
      }

      return new LoadTrustedSetupParameters(g1.array(), g1Count, g2.array(), g2Count);
    } catch (final IOException ex) {
      throw new UncheckedIOException(ex);
    }
  }

  private static UInt256 randomBLSFieldElement() {
    final BigInteger attempt = new BigInteger(CKZG4844JNI.BLS_MODULUS.bitLength(), RANDOM);
    if (attempt.compareTo(CKZG4844JNI.BLS_MODULUS) < 0) {
      return UInt256.valueOf(attempt);
    }
    return randomBLSFieldElement();
  }

  public static byte[] randomBLSFieldElementBytes() {
    return randomBLSFieldElement().toArray(ByteOrder.BIG_ENDIAN);
  }

  public static List<String> getFiles(String path) {
    try {
      try (Stream<Path> stream = Files.list(Paths.get(path))) {
        return stream.map(Path::toString).sorted().collect(Collectors.toList());
      }
    } catch (final IOException ex) {
      throw new UncheckedIOException(ex);
    }
  }

  public static List<String> getTestFiles(String path) {
    List<String> testFiles = new ArrayList<>();
    for (final String suite : getFiles(path)) {
      for (final String test : getFiles(suite)) {
        testFiles.addAll(getFiles(test));
      }
    }
    return testFiles;
  }
}
