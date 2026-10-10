class ThingReporter
{
	//! Reports a thing's progress.
	//! @route POST /api/v1/things/{id}/done|/api/v1/things/{id}/start
	//! @contract things.schema.json#/definitions/Done
	protected static void SendReport(notnull ThingCommand command, bool result)
	{
		string body = "{\"done\":true}";
	}

	//! @contract dup.schema.json
	void Ambiguous()
	{
	}

	//! @contract things.schema.json#/definitions/Missing
	void Broken()
	{
	}

	//! Shorthand alternation is not tag syntax: every alternative must be a whole template.
	//! @route POST /api/v1/things/{id}/done|start
	void Shorthand()
	{
	}
}
