//! What a refused delete of one object tells the operator. The delete is
//! conditional on the ETag they saw, so a refusal usually means the object moved
//! on — and that is worth saying in words rather than as a status.

import type { Failure } from "./screen.ts";

export function deleteRefusal(failure: Failure): Failure {
  switch (failure.status) {
    case 412:
      return { ...failure, message: "This object changed since you saw it, so it was not deleted." };
    case 404:
      return { ...failure, message: "This object is already gone." };
    default:
      return failure;
  }
}
