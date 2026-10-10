//! Reports thing progress to the API.
class ThingSender
{
	//! @route POST /api/v1/things/{id}/done|/api/v1/things/{id}/start
	//! @contract things.schema.json#/definitions/Done
	void SendDone(string id)
	{
		Print("sending " + id);
	}
}
