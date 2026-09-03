@echo off
rem Start the dev server. Sets up whatever is missing first, so a fresh clone
rem and an everyday run are the same command. The POSIX twin is run.sh.
rem
rem Every npm call needs `call`: npm is a .cmd, and a batch file that runs
rem another one without it hands over and never comes back, so the line after
rem would not run.

setlocal
cd /d "%~dp0web" || exit /b 1

if not exist "node_modules\" (
  echo Installing dependencies...
  call npm install || exit /b 1
)

rem The wasm build is half a minute and only core changes need it, so it runs
rem when the package is missing rather than every time. After editing anything
rem under crates\, run `npm run wasm` in web\ yourself.
if not exist "src\pkg\qubero_wasm.js" (
  where wasm-pack >nul 2>nul || (
    echo wasm-pack is not installed. Run: cargo install wasm-pack
    exit /b 1
  )
  rustup target list --installed 2>nul | findstr /c:"wasm32-unknown-unknown" >nul || (
    echo The wasm target is missing. Run: rustup target add wasm32-unknown-unknown
    exit /b 1
  )
  echo Building wasm...
  call npm run wasm || exit /b 1
)

rem The signatures that name which tool produced an executable. They are pinned
rem to one commit and downloaded, so this runs once per clone.
if not exist "public\diesig\pe.sig" (
  echo Downloading signatures...
  node ..\tools\die.mjs || exit /b 1
)

call npm run dev
