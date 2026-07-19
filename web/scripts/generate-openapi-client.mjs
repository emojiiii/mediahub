import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import openapiTS, { astToString } from "openapi-typescript";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const webRoot = path.resolve(scriptDirectory, "..");
const schemaPath = path.resolve(webRoot, "../openapi/openapi.json");
const outputPath = path.resolve(webRoot, "src/api/generated.ts");
const checkOnly = process.argv.includes("--check");

const nodes = await openapiTS(pathToFileURL(schemaPath));
const generated = [
  "// This file is generated from openapi/openapi.json. Do not edit it manually.",
  "/* eslint-disable */",
  "",
  astToString(nodes).trimEnd(),
  "",
].join("\n");

if (checkOnly) {
  let current;
  try {
    current = await readFile(outputPath, "utf8");
  } catch (error) {
    if (error?.code === "ENOENT") {
      console.error("Generated OpenAPI client is missing. Run npm run api:generate.");
      process.exitCode = 1;
    } else {
      throw error;
    }
  }

  if (current !== undefined && current !== generated) {
    console.error("Generated OpenAPI client is stale. Run npm run api:generate.");
    process.exitCode = 1;
  } else if (current !== undefined) {
    console.log("Generated OpenAPI client is up to date.");
  }
} else {
  await writeFile(outputPath, generated, "utf8");
  console.log(`Generated ${path.relative(webRoot, outputPath)}.`);
}
