export class OpenBrainError extends Error {
  public readonly code: string;
  public readonly status?: number;
  public readonly details?: unknown;

  constructor(
    code: string,
    message: string,
    opts?: { status?: number; details?: unknown }
  ) {
    super(message);
    this.name = "OpenBrainError";
    this.code = code;
    this.status = opts?.status;
    this.details = opts?.details;
  }
}
