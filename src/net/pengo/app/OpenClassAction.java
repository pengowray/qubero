package net.pengo.app;

import javax.swing.AbstractAction;
import java.awt.event.ActionEvent;
import net.pengo.data.*;
import java.io.*;
import java.util.zip.ZipFile;
import java.util.Enumeration;
import java.util.zip.ZipEntry;
import java.util.Random;

public class OpenClassAction extends AbstractAction {
    GUI gui;
    public OpenClassAction(GUI gui){
		super("Open a random Java Class...");
		this.gui = gui;
    }
    
    public void actionPerformed(ActionEvent e) {

        try {
            //Class guiclass = gui.getClass(); // GUI.class;
			/*
			// opens a serialized object (of a string's Class object)
            Class guiclass = String.class;
            ByteArrayOutputStream baos = new ByteArrayOutputStream();
            new ObjectOutputStream(baos).writeObject(guiclass);
            gui.open(new ArrayData(baos.toByteArray(), "Serialized Object: " + guiclass+""));
			 */
			
			//String cp = System.getProperty("java.class.path");
			//String cp = System.getProperty("java.library.path");
			//String cp = System.getProperty("java.ext.dirs");
			String cp = System.getProperty("sun.boot.class.path");
			//String cp = getClass().getClassLoader().getSystemResource("rt.jar").toString();
			String ps = System.getProperty("path.separator");
			String[] scp = cp.split(ps);
			String failed = "";
			//System.out.println("sun.boot.class.path: "+ cp);
			for (int i=0; i<scp.length; i++) {
				File dir = new File(scp[i]);
				File rtjar = dir;
				failed += "nondir:" + rtjar + "\r\n";
				//System.out.print(rtjar.getName() + ", ");
				//note: OSX (apple jvm 1.4.1_01) has "classes.jar".instead of "rt.jar"
				if (rtjar.getName().equalsIgnoreCase("rt.jar") || rtjar.getName().equalsIgnoreCase("classes.jar") ) {
					//gui.open(rtjar.getAbsolutePath());
					ZipFile zf = new ZipFile(rtjar);
					
					//choose a random entry.
					int totalEntries = zf.size();
					int count = (new Random().nextInt(totalEntries));
					
					Enumeration enumu = zf.entries();
					while (count > 1) {
						enumu.nextElement();
						count--;
					}
					ZipEntry ze = (ZipEntry)enumu.nextElement();
					gui.open(new SmallFileData(zf.getInputStream(ze),ze.getName()));
				}
			}
			//gui.open(new ArrayData(failed.getBytes(), "fail text"));
			/*
			// doesn't work
			if (getClass().getClassLoader().getSystemResource("java/lang") != null) {
				gui.open(new ArrayData(s.getBytes(), "worked"));
			} else {
				gui.open(new ArrayData(s.getBytes(), "didn't"));
			}
			 */
			
			
        } catch (IOException error) {
            gui.open(new ArrayData(error.getLocalizedMessage().getBytes(), "error"));
        }
        
        //String classname = this.getClass().getName()+ ".java";
        //String classname = new Object().getClass().getName();
        //String classname = this.getClass() + "";
        //String classname = "OpenClassAction";

		/*
        String classname = //"mooj32.png"; // works
            //"net/pengo/app/GUI.class"; // works
            "java/lang/String.class";
        InputStream is = getClass().getClassLoader().getSystemResourceAsStream(classname); // works
        //InputStream is = getClass().getResourceAsStream(classname);
		
        if (is == null) {
            String errormsg = "input stream null for " + classname;
            gui.open(new ArrayData(errormsg .getBytes()));
            return;
        }
        SmallFileData sfd = new SmallFileData(is, classname);
        gui.open(sfd);
		 */
    }
	
}

