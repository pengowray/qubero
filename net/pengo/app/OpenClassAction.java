package net.pengo.app;

import javax.swing.AbstractAction;
import java.awt.event.ActionEvent;
import java.awt.*;
import javax.swing.*;
import net.pengo.data.*;
import java.io.*;

public class OpenClassAction extends AbstractAction {
    GUI gui;
    public OpenClassAction(GUI gui){
	super("Open Java Class...");
	this.gui = gui;
    }
    
    public void actionPerformed(ActionEvent e) { 

        try {
            //Class guiclass = gui.getClass(); // GUI.class;
            Class guiclass = String.class; 
            ByteArrayOutputStream baos = new ByteArrayOutputStream();
            new ObjectOutputStream(baos).writeObject(guiclass);
            gui.open(new ArrayData(baos.toByteArray(), "Serialized Object: " + guiclass+""));
        } catch (IOException error) {
            gui.open(new ArrayData(error.getLocalizedMessage().getBytes(), "error"));
        }
        
        
        

        //String classname = this.getClass().getName()+ ".java";
        //String classname = new Object().getClass().getName();
        //String classname = this.getClass() + "";
        //String classname = "OpenClassAction";

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
    }
}

