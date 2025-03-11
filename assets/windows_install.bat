@echo off
setlocal

:: Check if version argument is provided
if "%1"=="" (
    echo Error: Please provide the version string as a command-line argument.
    echo Usage: windows_install.bat version
    exit /b 1
)

:: Define variables
set "VERSION=%1"  :: Version passed as a command-line argument
set "URL=https://github.com/sshwy/pku3b/releases/download/%VERSION%/pku3b-%VERSION%-x86_64-pc-windows-msvc.zip"
set "ZIP_FILE=%TEMP%\pku3b.zip"
set "EXTRACT_DIR=%TEMP%\pku3b"
set "SLUG=pku3b"  :: You can change this to a different slug for other apps
set "HOME_DIR=%USERPROFILE%\AppData\Local"  :: Default to AppData\Local
set "DEST_DIR=%HOME_DIR%\%SLUG%\bin"
set "EXE_FILE=%EXTRACT_DIR%\%SLUG%-%VERSION%-x86_64-pc-windows-msvc\%SLUG%.exe"
set "PATH_VAR=%HOME_DIR%\%SLUG%\bin"

:: Step 1: Download the file
echo Step 1: Downloading %SLUG% version %VERSION%...
powershell -Command "(New-Object System.Net.WebClient).DownloadFile('%URL%', '%ZIP_FILE%')"
if %errorlevel% neq 0 (
    echo Failed to download the file. Exiting...
    exit /b 1
)
echo Download complete.

:: Step 2: Extract the zip file
echo Step 2: Extracting %SLUG% version %VERSION%...
powershell -Command "Expand-Archive -Path '%ZIP_FILE%' -DestinationPath '%EXTRACT_DIR%' -Force"
if %errorlevel% neq 0 (
    echo Failed to extract the zip file. Exiting...
    exit /b 1
)
echo Extraction complete.

:: Step 3: Move %SLUG%.exe to the target directory
echo Step 3: Moving %SLUG%.exe to %DEST_DIR%...
if not exist "%DEST_DIR%" mkdir "%DEST_DIR%"
move /Y "%EXE_FILE%" "%DEST_DIR%\%SLUG%.exe"
if %errorlevel% neq 0 (
    echo Failed to move the executable. Exiting...
    exit /b 1
)
echo File moved to %DEST_DIR%.

:: Step 4: Add the directory to the PATH variable if it's not already there
echo Step 4: Checking if %PATH_VAR% is in the PATH variable...
set "CURRENT_PATH=%PATH%"
echo %CURRENT_PATH% | findstr /C:"%PATH_VAR%" >nul
if %errorlevel% neq 0 (
    echo %PATH_VAR% is not in the PATH variable. Adding it...
    setx PATH "%CURRENT_PATH%;%PATH_VAR%"
    if %errorlevel% neq 0 (
        echo Failed to update the PATH variable. Exiting...
        exit /b 1
    )
    echo PATH updated successfully.
) else (
    echo %PATH_VAR% is already in the PATH variable.
)

echo Installation complete!

endlocal
pause
