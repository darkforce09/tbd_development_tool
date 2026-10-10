// The web client.

// @route GET /api/v1/items/{itemId}
export async function loadItem(id: string): Promise<Response> {
  return fetch(`/api/v1/items/${id}`);
}

// @route DELETE /api/v1/things/{id}
export async function removeThing(id: string): Promise<Response> {
  return fetch(`/api/v1/things/${id}`, { method: "DELETE" });
}
