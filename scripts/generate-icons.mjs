// Generates viewer/src/icons.tsv: which Material Icon Theme icon each file and folder gets, and how it
// is drawn in the terminal (Nerd Font glyph + colour taken from Material's SVG).
//
//   node scripts/generate-icons.mjs [pkief.material-icon-theme folder]
//
// Without an argument it uses the latest version installed in ~/.vscode/extensions. The mappings
// (name → icon) are Material's as they are; the only hand-written part is GLYPHS, the Nerd Font
// drawing that looks most like each icon. When real SVGs can be drawn (Kitty graphics protocol),
// it is enough to swap the glyph for the image: the rest of the table stays valid.

import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

// Same version as the Nerd Fonts release the glyph codes come from: codes change between versions.
const NERD_FONTS = 'https://raw.githubusercontent.com/ryanoasis/nerd-fonts/v3.5.1/glyphnames.json';

// Material icon → Nerd Font glyph. Anything not listed is drawn with the generic file (or folder)
// glyph, but in the colour of its Material icon.
const GLYPHS = {
  typescript: 'seti-typescript', 'typescript-def': 'seti-typescript', tsconfig: 'seti-tsconfig',
  javascript: 'seti-javascript', react: 'dev-react', react_ts: 'dev-react',
  json: 'seti-json', markdown: 'seti-markdown', readme: 'seti-info', changelog: 'md-text_box_outline',
  license: 'seti-license', document: 'md-file_document_outline', table: 'md-file_table',
  html: 'seti-html', css: 'seti-css', sass: 'seti-sass', tailwindcss: 'dev-tailwindcss',
  astro: 'dev-astro', vue: 'seti-vue', svelte: 'seti-svelte', vite: 'md-lightning_bolt',
  python: 'seti-python', rust: 'seti-rust', go: 'seti-go', java: 'seti-java', c: 'seti-c',
  cpp: 'seti-cpp', csharp: 'seti-c_sharp', php: 'seti-php', ruby: 'seti-ruby', lua: 'seti-lua',
  kotlin: 'seti-kotlin', swift: 'seti-swift', dart: 'seti-dart', haskell: 'seti-haskell',
  elixir: 'seti-elixir', zig: 'seti-zig', julia: 'seti-julia', r: 'seti-r', scala: 'seti-scala',
  clojure: 'seti-clojure', graphql: 'seti-graphql', tex: 'seti-tex', jinja: 'seti-jinja',
  console: 'seti-shell', powershell: 'seti-powershell', makefile: 'seti-makefile',
  docker: 'seti-docker', git: 'seti-git', 'github-actions-workflow': 'seti-github', gitlab: 'seti-gitlab',
  yaml: 'seti-yml', toml: 'custom-toml', xml: 'md-xml', settings: 'seti-config', editorconfig: 'seti-config',
  tune: 'md-key', key: 'md-key', certificate: 'md-certificate', lock: 'seti-lock',
  nodejs: 'dev-nodejs_small', npm: 'seti-npm', yarn: 'seti-yarn', babel: 'seti-babel',
  webpack: 'seti-webpack', eslint: 'seti-eslint', prettier: 'md-format_paint', stylelint: 'seti-stylelint',
  database: 'seti-db', prisma: 'seti-prisma',
  image: 'seti-image', svg: 'seti-svg', favicon: 'seti-favicon', video: 'seti-video', audio: 'seti-audio',
  font: 'seti-font', pdf: 'seti-pdf', zip: 'seti-zip', exe: 'md-application', log: 'md-math_log',
  word: 'md-microsoft_word', powerpoint: 'md-microsoft_powerpoint', '3d': 'md-cube_outline',
  blender: 'md-blender_software', email: 'md-email',
};
const FILE_GLYPH = 'seti-default';
const TEST_GLYPH = 'md-test_tube'; // test-ts, test-js, test-jsx…
const FOLDER_GLYPH = 'md-folder';
const OPEN_FOLDER_GLYPH = 'md-folder_open';
const NO_FILL_COLOR = '#90a4ae'; // the grey of Material's generic icons

function installedMaterial() {
  const base = join(homedir(), '.vscode', 'extensions');
  const versions = readdirSync(base).filter((d) => d.startsWith('pkief.material-icon-theme-')).sort(
    (a, b) => a.localeCompare(b, undefined, { numeric: true }),
  );
  if (!versions.length) throw new Error(`No Material Icon Theme in ${base}; pass its folder as an argument.`);
  return join(base, versions.at(-1));
}

const material = process.argv[2] ?? installedMaterial();
const manifest = JSON.parse(readFileSync(join(material, 'dist', 'material-icons.json'), 'utf8'));
const version = JSON.parse(readFileSync(join(material, 'package.json'), 'utf8')).version;
const glyphNames = await (await fetch(NERD_FONTS)).json();

function code(glyphName) {
  const g = glyphNames[glyphName];
  if (!g) throw new Error(`Glyph “${glyphName}” does not exist in Nerd Fonts 3.5.1.`);
  return g.code;
}

// Main colour of the icon: the SVG's first hex fill (in Material, the body of the drawing).
function color(icon) {
  const svg = readFileSync(join(material, 'dist', manifest.iconDefinitions[icon].iconPath), 'utf8');
  const m = svg.match(/fill(?:="|:\s*)(#[0-9a-fA-F]{6}|#[0-9a-fA-F]{3})\b/);
  if (!m) return NO_FILL_COLOR;
  const hex = m[1].toLowerCase();
  return hex.length === 4 ? `#${hex[1]}${hex[1]}${hex[2]}${hex[2]}${hex[3]}${hex[3]}` : hex;
}

function glyph(icon) {
  if (icon.startsWith('folder')) return code(icon.endsWith('-open') ? OPEN_FOLDER_GLYPH : FOLDER_GLYPH);
  if (GLYPHS[icon]) return code(GLYPHS[icon]);
  if (icon.startsWith('test-')) return code(TEST_GLYPH);
  return code(FILE_GLYPH);
}

const unused = Object.keys(GLYPHS).filter((i) => !manifest.iconDefinitions[i]);
if (unused.length) console.warn(`Warning: these icons no longer exist in Material ${version}: ${unused.join(', ')}`);

const lines = [
  `# Generated by scripts/generate-icons.mjs from Material Icon Theme ${version} and Nerd Fonts 3.5.1. Do not edit by hand.`,
  '# I icon colour glyph · F file-name icon · E extension icon · D folder icon open-icon · X key icon',
];
for (const icon of Object.keys(manifest.iconDefinitions).sort()) {
  lines.push(`I\t${icon}\t${color(icon)}\t${glyph(icon)}`);
}
// Material compares in lower case; they are stored that way and the viewer looks them up the same way.
for (const [name, icon] of Object.entries(manifest.fileNames)) lines.push(`F\t${name.toLowerCase()}\t${icon}`);
for (const [ext, icon] of Object.entries(manifest.fileExtensions)) lines.push(`E\t${ext.toLowerCase()}\t${icon}`);
for (const [name, icon] of Object.entries(manifest.folderNames)) {
  lines.push(`D\t${name.toLowerCase()}\t${icon}\t${manifest.folderNamesExpanded[name] ?? `${icon}-open`}`);
}
for (const key of ['file', 'folder', 'folderExpanded', 'rootFolder', 'rootFolderExpanded']) {
  lines.push(`X\t${key}\t${manifest[key]}`);
}

const target = join(dirname(fileURLToPath(import.meta.url)), '..', 'viewer', 'src', 'icons.tsv');
writeFileSync(target, `${lines.join('\n')}\n`);
console.log(`${target}: ${Object.keys(manifest.iconDefinitions).length} icons, ${lines.length} lines.`);
