; Types (extensions add methods to an existing type)
(class_declaration declaration_kind: ["class" "struct" "actor"] name: (type_identifier) @class.name) @class.def
(class_declaration
  declaration_kind: ["class" "struct" "actor"]
  name: (type_identifier) @class.name
  (inheritance_specifier inherits_from: (user_type (type_identifier) @class.base))) @class.def
(class_declaration declaration_kind: "extension" name: (user_type (type_identifier) @impl.name)) @impl.def
(class_declaration declaration_kind: "enum" name: (type_identifier) @enum.name) @enum.def
(class_declaration
  declaration_kind: "enum"
  name: (type_identifier) @enum.name
  body: (enum_class_body (enum_entry name: (simple_identifier) @enum.variant))) @enum.def
(protocol_declaration name: (type_identifier) @iface.name) @iface.def

; Functions
(function_declaration name: (simple_identifier) @func.name) @func.def
(function_declaration name: (simple_identifier) @func.name (parameter) @func.param) @func.def
(function_declaration name: (simple_identifier) @func.name return_type: (_) @func.return) @func.def
(protocol_function_declaration name: (simple_identifier) @func.name) @func.def

; Properties
(property_declaration name: (pattern (simple_identifier) @field.name)) @field.def
(property_declaration name: (pattern (simple_identifier) @field.name) (type_annotation (_) @field.type)) @field.def

; Imports
(import_declaration (identifier) @import.path) @import.def

; Calls
(call_expression (simple_identifier) @call.name)
(call_expression (navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @call.name)))
