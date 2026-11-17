-- Turns one Word file into a PDF with Microsoft Word for Mac. Arguments, never
-- part of this text: the input path and the output path. The file opens
-- read-only, and Word is quit again only when this script was the one to start
-- it. The target is a literal app id so the compiler reads Word's scripting
-- dictionary; a variable target would leave its terms unknown.
-- Checked by hand with Word 16.114 for Mac.
on run argv
	set inPath to item 1 of argv
	set outPath to item 2 of argv
	set wasRunning to application id "com.microsoft.Word" is running
	try
		tell application id "com.microsoft.Word"
			open file name inPath read only true add to recent files false confirm conversions false
			set theDocument to active document
			try
				save as theDocument file name outPath file format format PDF
			on error errorText number errorNumber
				close theDocument saving no
				error errorText number errorNumber
			end try
			close theDocument saving no
		end tell
	on error errorText number errorNumber
		if not wasRunning then
			try
				tell application id "com.microsoft.Word" to quit
			end try
		end if
		error errorText number errorNumber
	end try
	if not wasRunning then
		try
			tell application id "com.microsoft.Word" to quit
		end try
	end if
end run
