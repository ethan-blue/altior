/**
 * Typed errors for Altior Desktop transport and IPC layer.
 */

export class CoreTransportError extends Error {
  readonly code: string;
  readonly details?: unknown;

  constructor(message: string, code = "TRANSPORT_ERROR", details?: unknown) {
    super(message);
    this.name = "CoreTransportError";
    this.code = code;
    this.details = details;
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

/**
 * Thrown when Tauri IPC or required capability is unavailable in the current runtime environment.
 */
export class TransportUnavailableError extends CoreTransportError {
  constructor(
    message = "Tauri IPC transport is unavailable in current runtime environment",
    details?: unknown,
  ) {
    super(message, "TRANSPORT_UNAVAILABLE", details);
    this.name = "TransportUnavailableError";
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

/**
 * Thrown when handshake negotiation fails or versions are incompatible.
 */
export class HandshakeError extends CoreTransportError {
  constructor(message: string, details?: unknown) {
    super(message, "HANDSHAKE_ERROR", details);
    this.name = "HandshakeError";
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

/**
 * Thrown when an operation is attempted on a closed or disconnected transport.
 */
export class ConnectionClosedError extends CoreTransportError {
  constructor(message = "Transport connection has been closed", details?: unknown) {
    super(message, "CONNECTION_CLOSED", details);
    this.name = "ConnectionClosedError";
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

/**
 * Thrown when a command is rejected or returns an error from Core.
 */
export class CommandError extends CoreTransportError {
  constructor(message: string, code = "COMMAND_ERROR", details?: unknown) {
    super(message, code, details);
    this.name = "CommandError";
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

/**
 * Thrown when receiving malformed envelopes, unhandled sequence gaps, or protocol violations.
 */
export class ProtocolError extends CoreTransportError {
  constructor(message: string, details?: unknown) {
    super(message, "PROTOCOL_ERROR", details);
    this.name = "ProtocolError";
    Object.setPrototypeOf(this, new.target.prototype);
  }
}
