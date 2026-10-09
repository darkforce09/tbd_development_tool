; Classes and modules
(class name: [(constant) (scope_resolution)] @class.name) @class.def
(class name: [(constant) (scope_resolution)] @class.name superclass: (superclass (_) @class.base)) @class.def
(module name: [(constant) (scope_resolution)] @class.name) @class.def

; Methods
(method name: (_) @func.name) @func.def
(method name: (_) @func.name parameters: (method_parameters) @func.params) @func.def
(singleton_method name: (_) @func.name) @func.def
(singleton_method name: (_) @func.name parameters: (method_parameters) @func.params) @func.def

; Imports
((call
  method: (identifier) @_req
  arguments: (argument_list (string (string_content) @import.path))) @import.def
  (#any-of? @_req "require" "require_relative" "load"))

; Calls
(call method: (identifier) @call.name)
