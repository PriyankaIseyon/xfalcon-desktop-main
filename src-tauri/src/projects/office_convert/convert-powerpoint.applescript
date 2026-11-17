-- Turns one PowerPoint file into a PDF with Microsoft PowerPoint for Mac.
-- Arguments, never part of this text: the input path and the output path.
-- PowerPoint is quit again only when this script was the one to start it. The
-- target is a literal app id so the compiler reads PowerPoint's scripting
-- dictionary; a variable target would leave its terms unknown.
-- Checked by hand with PowerPoint 16.114 for Mac. The PDF goes to a file
-- reference: given the output path as plain text, PowerPoint saves nothing
-- and reports no error.
on run argv
	set inPath to item 1 of argv
	set outPath to item 2 of argv
	set wasRunning to application id "com.microsoft.Powerpoint" is running
	try
		tell application id "com.microsoft.Powerpoint"
			open inPath
			set thePresentation to active presentation
			try
				save thePresentation in (POSIX file outPath) as save as PDF
			on error errorText number errorNumber
				close thePresentation saving no
				error errorText number errorNumber
			end try
			close thePresentation saving no
		end tell
	on error errorText number errorNumber
		if not wasRunning then
			try
				tell application id "com.microsoft.Powerpoint" to quit
			end try
		end if
		error errorText number errorNumber
	end try
	if not wasRunning then
		try
			tell application id "com.microsoft.Powerpoint" to quit
		end try
	end if
end run
