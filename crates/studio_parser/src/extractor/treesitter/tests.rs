use super::*;
use std::path::Path;

#[test]
fn all_lists_every_language_in_discriminant_order() {
    for (i, lang) in CodeLang::ALL.iter().enumerate() {
        assert_eq!(*lang as usize, i, "{lang:?}");
    }
}

#[test]
fn every_query_compiles() {
    let errors: Vec<String> = CodeLang::ALL
        .iter()
        .filter_map(|l| Query::new(&l.language(), l.query_source()).err().map(|e| format!("{}: {e}", l.name())))
        .collect();
    assert!(errors.is_empty(), "\n{}", errors.join("\n"));
}

fn extract(lang: CodeLang, ext: &str, src: &str) -> ExtractedFile {
    let path = format!("sample.{ext}");
    extract_code_file(lang, Path::new(&path), Path::new(&path), src)
}

fn method_names(file: &ExtractedFile, target: &str) -> Vec<String> {
    file.impls
        .iter()
        .filter(|i| i.target_type == target)
        .flat_map(|i| i.methods.iter().map(|m| m.name.clone()))
        .collect()
}

fn method<'a>(file: &'a ExtractedFile, target: &str, name: &str) -> &'a FunctionItem {
    file.impls
        .iter()
        .filter(|i| i.target_type == target)
        .flat_map(|i| i.methods.iter())
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("no method {target}::{name} in {:?}", file.impls))
}

/// Every snippet must be verbatim file text so the editor can splice edits back.
fn assert_verbatim(file: &ExtractedFile, src: &str) {
    let snippets = file
        .structs
        .iter()
        .map(|s| &s.source_code)
        .chain(file.enums.iter().map(|e| &e.source_code))
        .chain(file.traits.iter().map(|t| &t.source_code))
        .chain(file.functions.iter().map(|f| &f.source_code))
        .chain(file.impls.iter().flat_map(|i| i.methods.iter().map(|m| &m.source_code)));
    for s in snippets {
        assert!(src.contains(s.as_str()), "not verbatim:\n{s}");
    }
}

#[test]
fn python() {
    let code = r#"
import os
from pathlib import Path
from services.auth import verify_token

class Status(Enum):
    ACTIVE = 1
    INACTIVE = 2

class Pipeline(BaseEngine):
    def __init__(self):
        self.endpoint = "http://localhost"

    @property
    def run(self,
            retries: int = 3) -> bool:
        verify_token()
        def helper():
            cleanup()
        self.fetch_data("http://example.com")

# Fetches data.
async def fetch_data(url: str) -> dict:
    os.getenv("KEY")
"#;
    let file = extract(CodeLang::Python, "py", code);
    assert_verbatim(&file, code);

    assert_eq!(file.structs.len(), 1);
    assert_eq!(file.structs[0].name, "Pipeline");
    assert_eq!(file.structs[0].derives, ["BaseEngine"]);
    assert_eq!(file.structs[0].fields[0].name, "endpoint");

    assert_eq!(file.enums.len(), 1);
    assert_eq!(file.enums[0].name, "Status");
    assert_eq!(file.enums[0].variants, ["ACTIVE", "INACTIVE"]);

    assert_eq!(method_names(&file, "Pipeline"), ["__init__", "run"]);
    let run = method(&file, "Pipeline", "run");
    assert_eq!(run.calls, ["verify_token", "cleanup", "fetch_data"], "nested helper folds into run");
    assert_eq!(run.self_param.as_deref(), Some("self"));
    assert_eq!(run.inputs.len(), 1);
    assert_eq!(run.inputs[0].name, "retries");
    assert_eq!(run.inputs[0].type_str, "int");
    assert_eq!(run.output.as_deref(), Some("bool"));
    assert!(run.source_code.trim_start().starts_with("@property"), "decorator kept");

    assert_eq!(file.functions.len(), 1, "helper is nested, not top-level");
    let fetch = &file.functions[0];
    assert_eq!(fetch.name, "fetch_data");
    assert!(fetch.is_async);
    assert_eq!(fetch.calls, ["getenv"]);
    assert_eq!(fetch.docs, "Fetches data.");

    let uses: Vec<_> = file.uses.iter().map(|u| (u.path.as_str(), u.items.clone())).collect();
    assert_eq!(
        uses,
        [
            ("os", vec!["os".to_string()]),
            ("pathlib", vec!["Path".into()]),
            ("services.auth", vec!["verify_token".into()])
        ]
    );
}

#[test]
fn typescript() {
    let code = r#"
import { render, clear } from './view';

export enum EngineState {
    Ready,
    Running = 2,
    Stopped,
}

export interface CanvasProps {
    width: number;
    renderFrame(): void;
}

/** The engine. */
export class CanvasEngine extends BaseEngine implements Disposable {
    private frames: number = 0;

    renderFrame(): void {
        render();
        clear();
    }
}

export function startEngine(width: number,
                            height: number): CanvasEngine {
    render();
    return new CanvasEngine();
}

const stop = (engine: CanvasEngine) => engine.dispose();
"#;
    let file = extract(CodeLang::TypeScript, "ts", code);
    assert_verbatim(&file, code);

    assert_eq!(file.enums[0].name, "EngineState");
    assert_eq!(file.enums[0].variants, ["Ready", "Running", "Stopped"]);
    assert_eq!(file.traits[0].name, "CanvasProps");
    assert_eq!(file.traits[0].methods, ["renderFrame"]);

    let class = &file.structs[0];
    assert_eq!(class.name, "CanvasEngine");
    assert_eq!(class.derives, ["BaseEngine", "Disposable"]);
    assert_eq!(class.fields[0].name, "frames");
    assert_eq!(class.fields[0].visibility, ItemVisibility::Private);
    assert_eq!(class.docs, "The engine.");
    assert!(class.source_code.starts_with("export class"), "export wrapper kept");

    assert_eq!(method(&file, "CanvasEngine", "renderFrame").calls, ["render", "clear"]);

    let names: Vec<_> = file.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["startEngine", "stop"]);
    let start = &file.functions[0];
    assert_eq!(start.calls, ["render", "CanvasEngine"]);
    assert_eq!(start.inputs.len(), 2);
    assert_eq!(start.output.as_deref(), Some("CanvasEngine"));
    assert_eq!(file.functions[1].calls, ["dispose"]);

    assert_eq!(file.uses.len(), 1);
    assert_eq!(file.uses[0].path, "./view");
    assert_eq!(file.uses[0].items, ["render", "clear"]);
}

#[test]
fn javascript() {
    let code = "const fs = require('fs');\nimport def, { a } from \"./mod.js\";\nclass Store extends Base {\n  save(x) { fs.writeFileSync(x); }\n}\nfunction main() { new Store().save(1); }\n";
    let file = extract(CodeLang::JavaScript, "js", code);
    assert_verbatim(&file, code);
    assert_eq!(file.structs[0].derives, ["Base"]);
    assert_eq!(method(&file, "Store", "save").calls, ["writeFileSync"]);
    assert_eq!(file.functions[0].calls, ["Store", "save"]);
    let paths: Vec<_> = file.uses.iter().map(|u| u.path.as_str()).collect();
    assert_eq!(paths, ["fs", "./mod.js"]);
    assert_eq!(file.uses[1].items, ["def", "a"]);
}

#[test]
fn cpp() {
    let code = r#"
#include "renderer.h"
#include <vector>

enum class Quality { Low, High };

class Scene : public BaseScene {
public:
    void render() {
        drawGeometry();
        present();
    }
    int count;
};

void Scene::flush(int frames) {
    gpu.submit(frames);
}
"#;
    let file = extract(CodeLang::Cpp, "cpp", code);
    assert_verbatim(&file, code);
    assert_eq!(file.enums[0].name, "Quality");
    assert_eq!(file.enums[0].variants, ["Low", "High"]);
    assert_eq!(file.structs[0].name, "Scene");
    assert_eq!(file.structs[0].derives, ["BaseScene"]);
    assert_eq!(file.structs[0].fields[0].name, "count");
    assert_eq!(method_names(&file, "Scene"), ["render", "flush"], "out-of-line method attaches to its class");
    assert_eq!(method(&file, "Scene", "render").calls, ["drawGeometry", "present"]);
    assert_eq!(method(&file, "Scene", "flush").calls, ["submit"]);
    assert_eq!(file.uses.len(), 2);
    assert_eq!(file.uses[0].path, "renderer.h");
    assert_eq!(file.uses[0].items, ["renderer", "#include"]);
    assert_eq!(file.uses[1].path, "vector");
}

#[test]
fn c() {
    let code = "#include <stdio.h>\ntypedef struct { int x; } Point;\nenum Color { RED, GREEN };\nstatic int *make(int n) {\n  return calloc(n, 4);\n}\nint main(void) { printf(\"%d\", *make(1)); }\n";
    let file = extract(CodeLang::C, "c", code);
    assert_verbatim(&file, code);
    assert_eq!(file.structs[0].name, "Point");
    assert_eq!(file.structs[0].fields[0].name, "x");
    assert_eq!(file.enums[0].variants, ["RED", "GREEN"]);
    let names: Vec<_> = file.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["make", "main"]);
    assert_eq!(file.functions[1].calls, ["printf", "make"]);
}

#[test]
fn go() {
    let code = r#"
package engine

import (
    "fmt"
    "net/http"
)

type Dispatcher interface {
    Dispatch()
}

type Worker struct {
    ID int
}

func (w *Worker) Process() error {
    fmt.Println(w.ID)
    return nil
}

func helper(a, b int) {}
"#;
    let file = extract(CodeLang::Go, "go", code);
    assert_verbatim(&file, code);
    assert_eq!(file.traits[0].name, "Dispatcher");
    assert_eq!(file.traits[0].methods, ["Dispatch"]);
    assert_eq!(file.structs[0].name, "Worker");
    assert_eq!(file.structs[0].fields[0].name, "ID");
    assert_eq!(method(&file, "Worker", "Process").calls, ["Println"]);
    assert_eq!(method(&file, "Worker", "Process").output.as_deref(), Some("error"));
    assert_eq!(file.functions[0].name, "helper");
    assert_eq!(file.functions[0].visibility, ItemVisibility::Crate);
    let paths: Vec<_> = file.uses.iter().map(|u| (u.path.as_str(), u.items[0].as_str())).collect();
    assert_eq!(paths, [("fmt", "fmt"), ("net/http", "http")]);
}

#[test]
fn csharp() {
    let code = r#"
using System.Collections.Generic;

public enum OrderState {
    Pending,
    Shipped,
    Delivered
}

public class OrderService : ServiceBase, IDisposable {
    private int count;
    public string Name { get; set; }

    public void ProcessOrder(int orderId) {
        ValidateOrder(orderId);
        this.NotifyUser();
    }
}
"#;
    let file = extract(CodeLang::CSharp, "cs", code);
    assert_verbatim(&file, code);
    assert_eq!(file.uses[0].path, "System.Collections.Generic");
    assert_eq!(file.enums[0].variants, ["Pending", "Shipped", "Delivered"]);
    let class = &file.structs[0];
    assert_eq!(class.name, "OrderService");
    assert_eq!(class.derives, ["ServiceBase", "IDisposable"]);
    let fields: Vec<_> = class.fields.iter().map(|f| (f.name.as_str(), f.visibility)).collect();
    assert_eq!(fields, [("count", ItemVisibility::Private), ("Name", ItemVisibility::Public)]);
    let process = method(&file, "OrderService", "ProcessOrder");
    assert_eq!(process.calls, ["ValidateOrder", "NotifyUser"]);
    assert_eq!(process.inputs[0].name, "orderId");
    assert_eq!(process.inputs[0].type_str, "int");
}

#[test]
fn java() {
    let code = "import java.util.List;\npublic class Repo extends Base implements Store {\n  private final List<String> items;\n  /** Saves. */\n  public void save(String item) { items.add(item); log(item); }\n}\ninterface Store { void save(String item); }\nenum Mode { FAST, SAFE }\n";
    let file = extract(CodeLang::Java, "java", code);
    assert_verbatim(&file, code);
    assert_eq!(file.structs[0].derives, ["Base", "Store"]);
    assert_eq!(file.structs[0].fields[0].name, "items");
    assert_eq!(method(&file, "Repo", "save").calls, ["add", "log"]);
    assert_eq!(method(&file, "Repo", "save").docs, "Saves.");
    assert_eq!(file.traits[0].methods, ["save"]);
    assert_eq!(file.enums[0].variants, ["FAST", "SAFE"]);
    assert_eq!(file.uses[0].path, "java.util.List");
    assert_eq!(file.uses[0].items, ["List"]);
}

#[test]
fn kotlin() {
    let code = "import kotlinx.coroutines.launch\n\nclass Repo(val db: Db) : Base() {\n    fun load(id: Int): User {\n        return db.find(id)\n    }\n}\n\ninterface Api { fun get(): String }\n\nenum class Mode { FAST, SAFE }\n\nfun main() { launch { Repo(Db()).load(1) } }\n";
    let file = extract(CodeLang::Kotlin, "kt", code);
    assert_verbatim(&file, code);
    assert_eq!(file.structs.len(), 1);
    assert_eq!(file.structs[0].name, "Repo");
    assert_eq!(file.structs[0].derives, ["Base"]);
    assert_eq!(file.structs[0].fields[0].name, "db");
    assert_eq!(method(&file, "Repo", "load").calls, ["find"]);
    assert_eq!(file.traits[0].name, "Api");
    assert_eq!(file.enums[0].variants, ["FAST", "SAFE"]);
    assert_eq!(file.functions[0].name, "main");
    assert!(file.functions[0].calls.contains(&"load".to_string()));
    assert_eq!(file.uses[0].path, "kotlinx.coroutines.launch");
}

#[test]
fn swift() {
    let code = "import Foundation\n\nprotocol Drawable { func draw() }\n\nclass Shape: Drawable {\n    var name: String = \"\"\n    func draw() { render(name) }\n}\n\nextension Shape {\n    func area() -> Double { return compute() }\n}\n\nenum Kind { case circle, square }\n";
    let file = extract(CodeLang::Swift, "swift", code);
    assert_verbatim(&file, code);
    assert_eq!(file.traits[0].name, "Drawable");
    assert_eq!(file.traits[0].methods, ["draw"]);
    assert_eq!(file.structs.len(), 1, "extension is not a second type");
    assert_eq!(file.structs[0].derives, ["Drawable"]);
    assert_eq!(file.structs[0].fields[0].name, "name");
    assert_eq!(method_names(&file, "Shape"), ["draw", "area"]);
    assert_eq!(method(&file, "Shape", "area").calls, ["compute"]);
    assert_eq!(file.enums[0].variants, ["circle", "square"]);
    assert_eq!(file.uses[0].path, "Foundation");
}

#[test]
fn php() {
    let code = "<?php\nnamespace App;\nuse App\\Models\\User;\n\nclass UserService extends Service {\n    private $repo;\n    public function find(int $id): ?User {\n        return $this->repo->load($id);\n    }\n}\n\nfunction helper() { return strlen('x'); }\n";
    let file = extract(CodeLang::Php, "php", code);
    assert_verbatim(&file, code);
    assert_eq!(file.structs[0].name, "UserService");
    assert_eq!(file.structs[0].derives, ["Service"]);
    assert_eq!(file.structs[0].fields[0].name, "repo");
    assert_eq!(method(&file, "UserService", "find").calls, ["load"]);
    assert_eq!(file.functions[0].calls, ["strlen"]);
    assert_eq!(file.uses[0].path, "App\\Models\\User");
    assert_eq!(file.uses[0].items, ["User"]);
}

#[test]
fn ruby() {
    let code = "require 'json'\nrequire_relative 'base'\n\nmodule Billing\n  class Invoice < Base\n    def total(tax)\n      compute(tax)\n    end\n  end\nend\n";
    let file = extract(CodeLang::Ruby, "rb", code);
    assert_verbatim(&file, code);
    let names: Vec<_> = file.structs.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Billing", "Invoice"]);
    assert_eq!(file.structs[1].derives, ["Base"]);
    assert_eq!(method(&file, "Invoice", "total").calls, ["compute"]);
    let paths: Vec<_> = file.uses.iter().map(|u| u.path.as_str()).collect();
    assert_eq!(paths, ["json", "base"]);
}

#[test]
fn lua() {
    let code = "local json = require(\"json\")\nlocal M = {}\n\nfunction M.load(path)\n  return json.decode(read(path))\nend\n\nfunction M:save()\n  write(self)\nend\n\nlocal function helper() end\n";
    let file = extract(CodeLang::Lua, "lua", code);
    assert_verbatim(&file, code);
    assert_eq!(method_names(&file, "M"), ["load", "save"]);
    assert_eq!(method(&file, "M", "load").calls, ["decode", "read"]);
    assert_eq!(file.functions[0].name, "helper");
    assert_eq!(file.uses[0].path, "json");
}

#[test]
fn scala() {
    let code = "import scala.util.Try\n\ntrait Shape { def area: Double }\n\nclass Circle(r: Double) extends Shape {\n  val pi = 3.14\n  def area: Double = compute(r)\n}\n\nobject Main {\n  def main(args: Array[String]): Unit = println(new Circle(1).area)\n}\n";
    let file = extract(CodeLang::Scala, "scala", code);
    assert_verbatim(&file, code);
    assert_eq!(file.traits[0].name, "Shape");
    assert_eq!(file.traits[0].methods, ["area"]);
    let names: Vec<_> = file.structs.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Circle", "Main"]);
    assert_eq!(file.structs[0].derives, ["Shape"]);
    assert_eq!(file.structs[0].fields[0].name, "pi");
    assert_eq!(method(&file, "Circle", "area").calls, ["compute"]);
    assert_eq!(method(&file, "Main", "main").calls, ["println"]);
    assert_eq!(file.uses[0].path, "scala.util.Try");
}

#[test]
fn dart() {
    let code = "import 'package:flutter/material.dart';\n\nclass Counter extends Base {\n  int increment(int by) {\n    return add(by);\n  }\n}\n\nenum Mode { fast, safe }\n\nvoid main() {\n  runApp(Counter());\n}\n";
    let file = extract(CodeLang::Dart, "dart", code);
    assert_verbatim(&file, code);
    assert_eq!(file.structs[0].name, "Counter");
    assert_eq!(file.structs[0].derives, ["Base"]);
    assert_eq!(method(&file, "Counter", "increment").calls, ["add"]);
    assert_eq!(file.enums[0].variants, ["fast", "safe"]);
    assert_eq!(file.functions[0].name, "main");
    assert_eq!(file.functions[0].calls, ["runApp", "Counter"]);
    assert_eq!(file.uses[0].path, "package:flutter/material.dart");
}

#[test]
fn zig() {
    let code = "const std = @import(\"std\");\n\nconst Point = struct {\n    x: i32,\n    pub fn len(self: Point) i32 {\n        return std.math.sqrt(self.x);\n    }\n};\n\nconst Mode = enum { fast, safe };\n\npub fn main() void {\n    helper();\n}\n";
    let file = extract(CodeLang::Zig, "zig", code);
    assert_verbatim(&file, code);
    assert_eq!(file.structs[0].name, "Point");
    assert_eq!(file.structs[0].fields[0].name, "x");
    assert_eq!(method(&file, "Point", "len").calls, ["sqrt"]);
    assert_eq!(file.enums[0].variants, ["fast", "safe"]);
    assert_eq!(file.functions[0].name, "main");
    assert_eq!(file.functions[0].calls, ["helper"]);
    assert_eq!(file.uses[0].path, "std");
}

#[test]
fn bash() {
    let code = "#!/bin/bash\nsource ./lib.sh\n\ndeploy() {\n  build_image\n  push_image \"$1\"\n}\n\nfunction build_image {\n  docker build .\n}\n";
    let file = extract(CodeLang::Bash, "sh", code);
    assert_verbatim(&file, code);
    let names: Vec<_> = file.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["deploy", "build_image"]);
    assert_eq!(file.functions[0].calls, ["build_image", "push_image"]);
    assert_eq!(file.uses[0].path, "./lib.sh");
}

#[test]
fn crlf_snippets_are_lf_and_lines_are_correct() {
    let code = "def a():\r\n    pass\r\n\r\ndef b():\r\n    a()\r\n";
    let file = extract(CodeLang::Python, "py", code);
    assert_eq!(file.functions[1].source_code, "def b():\n    a()");
    assert_eq!(file.functions[1].line, 4);
    assert_verbatim(&file, &code.replace("\r\n", "\n"));
}

#[test]
fn syntax_errors_still_extract_and_minified_files_are_skipped() {
    let broken = "def ok():\n    pass\n\ndef broken(:\n";
    let file = extract(CodeLang::Python, "py", broken);
    assert!(file.parse_error.is_some());
    assert_eq!(file.functions[0].name, "ok");

    let minified = format!("{}\n", "var a=1;".repeat(100));
    assert!(extract(CodeLang::JavaScript, "js", &minified.repeat(10)).functions.is_empty());
}
