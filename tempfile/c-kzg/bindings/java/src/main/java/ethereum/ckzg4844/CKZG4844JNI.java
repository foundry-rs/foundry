package ethereum.ckzg4844;

import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
import java.math.BigInteger;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;

public class CKZG4844JNI {

  private static final String LIBRARY_NAME = "ckzg4844jni";
  private static final String PLATFORM_NATIVE_LIBRARY_NAME = System.mapLibraryName(LIBRARY_NAME);

  /** Loads the appropriate native library based on your platform. */
  public static void loadNativeLibrary() {
    String libraryResourcePath =
        "lib/" + System.getProperty("os.arch") + "/" + PLATFORM_NATIVE_LIBRARY_NAME;
    InputStream libraryResource = CKZG4844JNI.class.getResourceAsStream(libraryResourcePath);
    if (libraryResource == null) {
      try {
        System.loadLibrary(LIBRARY_NAME);
      } catch (UnsatisfiedLinkError __) {
        String exceptionMessage =
            String.format(
                "Couldn't load native library (%s). It wasn't available at %s or the library path.",
                LIBRARY_NAME, libraryResourcePath);
        throw new RuntimeException(exceptionMessage);
      }
    } else {
      try {
        Path tempDir = Files.createTempDirectory(LIBRARY_NAME + "@");
        tempDir.toFile().deleteOnExit();
        Path tempDll = tempDir.resolve(PLATFORM_NATIVE_LIBRARY_NAME);
        tempDll.toFile().deleteOnExit();
        Files.copy(libraryResource, tempDll, StandardCopyOption.REPLACE_EXISTING);
        libraryResource.close();
        System.load(tempDll.toString());
      } catch (IOException ex) {
        throw new UncheckedIOException(ex);
      }
    }
  }

  /** Scalar field modulus of BLS12-381 */
  public static final BigInteger BLS_MODULUS =
      new BigInteger(
          "52435875175126190479447740508185965837690552500527637822603658699938581184513");
  /** The number of bytes in a g1 point. */
  public static final int BYTES_PER_G1 = 48;
  /** The number of bytes in a g2 point. */
  public static final int BYTES_PER_G2 = 96;
  /** The number of bytes in a KZG commitment */
  public static final int BYTES_PER_COMMITMENT = 48;
  /** The number of bytes in a KZG proof */
  public static final int BYTES_PER_PROOF = 48;
  /** Bytes used to encode a BLS scalar field element */
  public static final int BYTES_PER_FIELD_ELEMENT = 32;
  /** Number of field elements in a blob */
  public static final int FIELD_ELEMENTS_PER_BLOB = 4096;
  /** Number of field elements in a blob */
  public static final int BYTES_PER_BLOB = FIELD_ELEMENTS_PER_BLOB * BYTES_PER_FIELD_ELEMENT;

  private CKZG4844JNI() {}

  /**
   * Loads the trusted setup from a file. Once loaded, the same setup will be used for all the
   * crypto native calls. To load a new setup, free the current one by calling {@link
   * #freeTrustedSetup()} and then load the new one. If no trusted setup has been loaded, all the
   * crypto native calls will throw a {@link RuntimeException}.
   *
   * @param file a path to a trusted setup file
   * @throws CKZGException if there is a crypto error
   */
  public static native void loadTrustedSetup(String file);

  /**
   * An alternative to {@link #loadTrustedSetup(String)}. Loads the trusted setup from method
   * parameters instead of a file.
   *
   * @param g1 g1 values as bytes
   * @param g1Count the count of the g1 values
   * @param g2 g2 values as bytes
   * @param g2Count the count of the g2 values
   * @throws CKZGException if there is a crypto error
   */
  public static native void loadTrustedSetup(byte[] g1, long g1Count, byte[] g2, long g2Count);

  /**
   * An alternative to {@link #loadTrustedSetup(String)}. Loads the trusted setup from a resource.
   *
   * @param resource the resource name that contains the trusted setup
   * @param clazz the class to use to get the resource
   * @param <T> the type of the class
   * @throws CKZGException if there is a crypto error
   * @throws IllegalArgumentException if the resource does not exist
   */
  public static <T> void loadTrustedSetupFromResource(String resource, Class<T> clazz) {
    InputStream is = clazz.getResourceAsStream(resource);
    if (is == null) {
      throw new IllegalArgumentException("Resource " + resource + " does not exist.");
    }

    try (is) {
      Path jniWillLoadFrom = Files.createTempFile("kzg-trusted-setup", ".txt");
      jniWillLoadFrom.toFile().deleteOnExit();
      Files.copy(is, jniWillLoadFrom, StandardCopyOption.REPLACE_EXISTING);
      loadTrustedSetup(jniWillLoadFrom.toString());
    } catch (IOException ex) {
      throw new UncheckedIOException("Error loading trusted setup from resource " + resource, ex);
    }
  }

  /**
   * Free the current trusted setup. This method will throw an exception if no trusted setup has
   * been loaded.
   */
  public static native void freeTrustedSetup();

  /**
   * Calculates commitment for a given blob
   *
   * @param blob blob bytes
   * @return the commitment
   * @throws CKZGException if there is a crypto error
   */
  public static native byte[] blobToKzgCommitment(byte[] blob);

  /**
   * Compute proof at point z for the polynomial represented by blob.
   *
   * @param blob blob bytes
   * @param z_bytes a point
   * @return an instance of {@link ProofAndY} holding the proof and the value y = f(z)
   * @throws CKZGException if there is a crypto error
   */
  public static native ProofAndY computeKzgProof(byte[] blob, byte[] z_bytes);

  /**
   * Given a blob, return the KZG proof that is used to verify it against the commitment
   *
   * @param blob blob bytes
   * @param commitment_bytes commitment bytes
   * @return the proof
   * @throws CKZGException if there is a crypto error
   */
  public static native byte[] computeBlobKzgProof(byte[] blob, byte[] commitment_bytes);

  /**
   * Verify the proof by point evaluation for the given commitment
   *
   * @param commitment_bytes commitment bytes
   * @param z_bytes Z
   * @param y_bytes Y
   * @param proof_bytes the proof that needs verifying
   * @return true if the proof is valid and false otherwise
   * @throws CKZGException if there is a crypto error
   */
  public static native boolean verifyKzgProof(
      byte[] commitment_bytes, byte[] z_bytes, byte[] y_bytes, byte[] proof_bytes);

  /**
   * Given a blob and a KZG proof, verify that the blob data corresponds to the provided commitment.
   *
   * @param blob blob bytes
   * @param commitment_bytes commitment bytes
   * @param proof_bytes proof bytes
   * @return true if the proof is valid and false otherwise
   * @throws CKZGException if there is a crypto error
   */
  public static native boolean verifyBlobKzgProof(
      byte[] blob, byte[] commitment_bytes, byte[] proof_bytes);

  /**
   * Given a list of blobs and blob KZG proofs, verify that they correspond to the provided
   * commitments.
   *
   * @param blobs flattened blobs bytes
   * @param commitments_bytes flattened commitments bytes
   * @param proofs_bytes flattened proofs bytes
   * @param count the number of blobs (should be same as the number of proofs and commitments)
   * @return true if the proof is valid and false otherwise
   * @throws CKZGException if there is a crypto error
   */
  public static native boolean verifyBlobKzgProofBatch(
      byte[] blobs, byte[] commitments_bytes, byte[] proofs_bytes, long count);
}
