; Types
(class_declaration name: (identifier) @class.name) @class.def
(class_declaration name: (identifier) @class.name (base_list (_) @class.base)) @class.def
(struct_declaration name: (identifier) @class.name) @class.def
(struct_declaration name: (identifier) @class.name (base_list (_) @class.base)) @class.def
(record_declaration name: (identifier) @class.name) @class.def
(interface_declaration name: (identifier) @iface.name) @iface.def
(enum_declaration name: (identifier) @enum.name) @enum.def
(enum_declaration
  name: (identifier) @enum.name
  body: (enum_member_declaration_list (enum_member_declaration name: (identifier) @enum.variant))) @enum.def

; Members
(method_declaration
  returns: (_) @func.return
  name: (identifier) @func.name
  parameters: (parameter_list) @func.params) @func.def
(constructor_declaration name: (identifier) @func.name parameters: (parameter_list) @func.params) @func.def
(local_function_statement name: (identifier) @func.name parameters: (parameter_list) @func.params) @func.def
(field_declaration
  (variable_declaration type: (_) @field.type (variable_declarator name: (identifier) @field.name))) @field.def
(property_declaration type: (_) @field.type name: (identifier) @field.name) @field.def

; Imports
(using_directive [(identifier) (qualified_name)] @import.path) @import.def

; Calls
(invocation_expression function: (identifier) @call.name)
(invocation_expression function: (member_access_expression name: (identifier) @call.name))
(invocation_expression function: (generic_name (identifier) @call.name))
(object_creation_expression type: (identifier) @call.name)
