; Classes, objects, interfaces, enums (specific kinds win over plain class for the same node)
(class_declaration name: (identifier) @class.name) @class.def
(class_declaration
  name: (identifier) @class.name
  (delegation_specifiers (delegation_specifier [(user_type (identifier) @class.base)
                                                (constructor_invocation (user_type (identifier) @class.base))]))) @class.def
(object_declaration name: (identifier) @class.name) @class.def
(class_declaration "interface" name: (identifier) @iface.name) @iface.def
(class_declaration name: (identifier) @enum.name (enum_class_body)) @enum.def
(class_declaration
  name: (identifier) @enum.name
  (enum_class_body (enum_entry (identifier) @enum.variant))) @enum.def

; Functions
(function_declaration name: (identifier) @func.name (function_value_parameters) @func.params) @func.def

; Properties
(property_declaration (variable_declaration (identifier) @field.name)) @field.def
(class_parameter (identifier) @field.name) @field.def

; Imports
(import [(qualified_identifier) (identifier)] @import.path) @import.def

; Calls
(call_expression . (identifier) @call.name)
(call_expression . (navigation_expression (identifier) @call.name .))
