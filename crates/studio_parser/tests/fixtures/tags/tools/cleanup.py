import requests


# @route DELETE /api/v1/things/{id}
def delete_thing(thing_id):
    """@route GET /docstring-is-not-a-tag"""
    return requests.delete(f"/api/v1/things/{thing_id}")


# @contract things.schema.json#/
