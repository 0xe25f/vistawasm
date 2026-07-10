import type { VistaErrorCode } from "./types.js";

/**
 * Error class used by the public VistaWASM TypeScript API.
 */
export class VistaWasmError extends Error {
  public readonly code: VistaErrorCode;

  public readonly details?: unknown;

  public constructor(code: VistaErrorCode, message: string, details?: unknown) {
    super(message);
    this.name = "VistaWasmError";
    this.code = code;
    this.details = details;
  }
}

/**
 * Convert an unknown thrown value into a VistaWASM error.
 */
export function toVistaWasmError(error: unknown): VistaWasmError {
  if (error instanceof VistaWasmError) {
    return error;
  }

  if (isObject(error) && typeof error.code === "string" && typeof error.message === "string") {
    return new VistaWasmError(error.code as VistaErrorCode, error.message, error.details);
  }

  if (error instanceof Error) {
    return new VistaWasmError("INTERNAL_ERROR", error.message, error);
  }

  return new VistaWasmError("INTERNAL_ERROR", "VistaWASM encountered an internal fault.", error);
}

function isObject(value: unknown): value is { code?: unknown; message?: unknown; details?: unknown } {
  return typeof value === "object" && value !== null;
}
