-- Turns one Excel file into a PDF with Microsoft Excel for Mac. Arguments,
-- never part of this text: the input path and the output path. The workbook
-- opens read-only, and Excel is quit again only when this script was the one to
-- start it. The target is a literal app id so the compiler reads Excel's
-- scripting dictionary; a variable target would leave its terms unknown.
-- Checked by hand with Excel 16.114 for Mac.
on run argv
	set inPath to item 1 of argv
	set outPath to item 2 of argv
	set wasRunning to application id "com.microsoft.Excel" is running
	try
		tell application id "com.microsoft.Excel"
			open workbook workbook file name inPath read only true
			set theWorkbook to active workbook
			try
				save workbook as theWorkbook filename outPath file format PDF file format
			on error errorText number errorNumber
				close theWorkbook saving no
				error errorText number errorNumber
			end try
			close theWorkbook saving no
		end tell
	on error errorText number errorNumber
		if not wasRunning then
			try
				tell application id "com.microsoft.Excel" to quit
			end try
		end if
		error errorText number errorNumber
	end try
	if not wasRunning then
		try
			tell application id "com.microsoft.Excel" to quit
		end try
	end if
end run
