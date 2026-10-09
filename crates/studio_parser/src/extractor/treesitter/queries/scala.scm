; Types
(class_definition name: (identifier) @class.name) @class.def
(class_definition name: (identifier) @class.name extend: (extends_clause type: (_) @class.base)) @class.def
(object_definition name: (identifier) @class.name) @class.def
(trait_definition name: (identifier) @iface.name) @iface.def
(enum_definition name: (identifier) @enum.name) @enum.def
(enum_definition name: (identifier) @enum.name body: (enum_body (enum_case_definitions (simple_enum_case name: (identifier) @enum.variant)))) @enum.def

; Functions
(function_definition name: (identifier) @func.name) @func.def
(function_definition name: (identifier) @func.name parameters: (parameters) @func.params) @func.def
(function_definition name: (identifier) @func.name parameters: (parameters) @func.params return_type: (_) @func.return) @func.def
(function_declaration name: (identifier) @func.name) @func.def
(function_declaration name: (identifier) @func.name parameters: (parameters) @func.params) @func.def

; Fields
(template_body (val_definition pattern: (identifier) @field.name) @field.def)
(template_body (val_definition pattern: (identifier) @field.name type: (_) @field.type) @field.def)
(template_body (var_definition pattern: (identifier) @field.name) @field.def)

; Imports
(import_declaration) @import.def

; Calls
(call_expression function: (identifier) @call.name)
(call_expression function: (field_expression field: (identifier) @call.name))
