; Types
(class_declaration name: (identifier) @class.name) @class.def
(class_declaration name: (identifier) @class.name superclass: (superclass (_) @class.base)) @class.def
(class_declaration name: (identifier) @class.name interfaces: (super_interfaces (type_list (_) @class.base))) @class.def
(record_declaration name: (identifier) @class.name) @class.def
(interface_declaration name: (identifier) @iface.name) @iface.def
(enum_declaration name: (identifier) @enum.name) @enum.def
(enum_declaration
  name: (identifier) @enum.name
  body: (enum_body (enum_constant name: (identifier) @enum.variant))) @enum.def

; Members
(method_declaration
  type: (_) @func.return
  name: (identifier) @func.name
  parameters: (formal_parameters) @func.params) @func.def
(constructor_declaration name: (identifier) @func.name parameters: (formal_parameters) @func.params) @func.def
(field_declaration type: (_) @field.type declarator: (variable_declarator name: (identifier) @field.name)) @field.def

; Imports
(import_declaration [(scoped_identifier) (identifier)] @import.path) @import.def

; Calls
(method_invocation name: (identifier) @call.name)
(object_creation_expression type: (type_identifier) @call.name)
