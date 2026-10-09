; Types
(type_declaration (type_spec name: (type_identifier) @class.name type: (struct_type)) @class.def)
(type_declaration (type_spec name: (type_identifier) @iface.name type: (interface_type)) @iface.def)
(field_declaration name: (field_identifier) @field.name type: (_) @field.type) @field.def

; Functions and methods (methods attach to their receiver type)
(function_declaration
  name: (identifier) @func.name
  parameters: (parameter_list) @func.params) @func.def
(function_declaration
  name: (identifier) @func.name
  parameters: (parameter_list) @func.params
  result: (_) @func.return) @func.def
(method_declaration
  receiver: (parameter_list
    (parameter_declaration type: [(type_identifier) @func.receiver
                                  (pointer_type (type_identifier) @func.receiver)
                                  (generic_type type: (type_identifier) @func.receiver)
                                  (pointer_type (generic_type type: (type_identifier) @func.receiver))]))
  name: (field_identifier) @func.name
  parameters: (parameter_list) @func.params) @func.def
(method_declaration
  receiver: (parameter_list
    (parameter_declaration type: [(type_identifier) @func.receiver
                                  (pointer_type (type_identifier) @func.receiver)]))
  name: (field_identifier) @func.name
  parameters: (parameter_list) @func.params
  result: (_) @func.return) @func.def
(method_elem name: (field_identifier) @func.name parameters: (parameter_list) @func.params) @func.def

; Imports
(import_spec path: (interpreted_string_literal) @import.path) @import.def

; Calls
(call_expression function: (identifier) @call.name)
(call_expression function: (selector_expression field: (field_identifier) @call.name))
