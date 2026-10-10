import { get } from "./http";

/** @route GET /api/v1/things */
export function listThings() {
  return get("/api/v1/things");
}
