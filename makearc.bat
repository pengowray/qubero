rem "$(date +%F)"  <-- doesn't work

jar -cvfm "$(date +%F) project-moojasm.jar" mf.txt -C . *.class 
jar -uvf "$(date +%F) project-moojasm.jar" -C . *.java 
jar -uvf "$(date +%F) project-moojasm.jar" -C . mooj32.png
jar -uvf "$(date +%F) project-moojasm.jar" -C "todo" todo/*.*
