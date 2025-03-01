package ethereum.ckzg4844;

import static ethereum.ckzg4844.CKZGException.CKZGError.fromErrorCode;

import java.util.Arrays;

/** Thrown when there is an error in the underlying c-kzg library. */
public class CKZGException extends RuntimeException {

  private final CKZGError error;
  private final String errorMessage;

  public CKZGException(int errorCode, String errorMessage) {
    super(String.format("%s (%s)", errorMessage, fromErrorCode(errorCode)));
    this.error = fromErrorCode(errorCode);
    this.errorMessage = errorMessage;
  }

  public CKZGError getError() {
    return error;
  }

  public String getErrorMessage() {
    return errorMessage;
  }

  public enum CKZGError {
    UNKNOWN(0),
    C_KZG_BADARGS(1),
    C_KZG_ERROR(2),
    C_KZG_MALLOC(3);

    public final int errorCode;

    CKZGError(int errorCode) {
      this.errorCode = errorCode;
    }

    public static CKZGError fromErrorCode(int errorCode) {
      return Arrays.stream(CKZGError.values())
          .filter(error -> error.errorCode == errorCode)
          .findFirst()
          .orElse(UNKNOWN);
    }
  }
}
