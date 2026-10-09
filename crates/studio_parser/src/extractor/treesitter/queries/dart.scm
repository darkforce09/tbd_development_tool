; Types
(class_declaration name: (identifier) @class.name) @class.def
(class_declaration name: (identifier) @class.name superclass: (superclass type: (_) @class.base)) @class.def
(mixin_declaration name: (identifier) @class.name) @class.def
(enum_declaration name: (identifier) @enum.name) @enum.def
(enum_declaration name: (identifier) @enum.name body: (enum_body (enum_constant name: (identifier) @enum.variant))) @enum.def

; Functions and methods
(function_declaration
  signature: (function_signature name: (identifier) @func.name parameters: (formal_parameter_list) @func.params)) @func.def
(function_declaration
  signature: (function_signature
    return_type: (_) @func.return
    name: (identifier) @func.name
    parameters: (formal_parameter_list) @func.params)) @func.def
(method_declaration
  signature: (method_signature
    (function_signature name: (identifier) @func.name parameters: (formal_parameter_list) @func.params))) @func.def
(method_declaration
  signature: (method_signature
    (function_signature
      return_type: (_) @func.return
      name: (identifier) @func.name
      parameters: (formal_parameter_list) @func.params))) @func.def
(method_declaration
  signature: (method_signature (constructor_signature name: (identifier) @func.name parameters: (formal_parameter_list) @func.params))) @func.def
(method_declaration
  signature: (method_signature (getter_signature name: (identifier) @func.name))) @func.def
(declaration (function_signature name: (identifier) @func.name parameters: (formal_parameter_list) @func.params)) @func.def

; Imports
(import_specification uri: (_) @import.path) @import.def

; Calls
(call_expression function: (identifier) @call.name)
(call_expression function: (member_expression property: (identifier) @call.name))
