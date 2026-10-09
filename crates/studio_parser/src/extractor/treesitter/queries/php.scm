; Types
(class_declaration name: (name) @class.name) @class.def
(class_declaration name: (name) @class.name (base_clause [(name) (qualified_name)] @class.base)) @class.def
(class_declaration name: (name) @class.name (class_interface_clause [(name) (qualified_name)] @class.base)) @class.def
(trait_declaration name: (name) @class.name) @class.def
(interface_declaration name: (name) @iface.name) @iface.def
(enum_declaration name: (name) @enum.name) @enum.def
(enum_declaration
  name: (name) @enum.name
  body: (enum_declaration_list (enum_case name: (name) @enum.variant))) @enum.def

; Functions and methods
(function_definition name: (name) @func.name parameters: (formal_parameters) @func.params) @func.def
(function_definition
  name: (name) @func.name
  parameters: (formal_parameters) @func.params
  return_type: (_) @func.return) @func.def
(method_declaration name: (name) @func.name parameters: (formal_parameters) @func.params) @func.def
(method_declaration
  name: (name) @func.name
  parameters: (formal_parameters) @func.params
  return_type: (_) @func.return) @func.def

; Properties
(property_declaration (property_element (variable_name (name) @field.name))) @field.def
(property_declaration type: (_) @field.type (property_element (variable_name (name) @field.name))) @field.def

; Imports
(namespace_use_clause [(name) (qualified_name)] @import.path) @import.def
((include_expression (string (string_content) @import.path)) @import.def)
((require_once_expression (string (string_content) @import.path)) @import.def)

; Calls
(function_call_expression function: [(name) (qualified_name)] @call.name)
(member_call_expression name: (name) @call.name)
(scoped_call_expression name: (name) @call.name)
(object_creation_expression [(name) (qualified_name)] @call.name)
