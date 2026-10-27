# sovra-skills

Local skill package IO behind `sovra_core::skills::SkillStore`.

- Runtime owns blocking calls. Do not add async or UI dependencies.
- Skills are procedures, never permission grants. Do not execute bundled scripts.
- Bound package discovery, YAML parsing and resource reads; reject symlinks and
  resource paths outside the installed package.
- Keep writes atomic and private. Preserve resources when editing instructions.
- Never log instruction/resource contents or copy credentials into skill packages.
