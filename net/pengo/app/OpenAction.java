package net.pengo.app;

import javax.swing.AbstractAction;
import java.awt.event.ActionEvent;
import java.awt.*;
import javax.swing.*;
import net.pengo.data.*;

public class OpenAction extends AbstractAction {
    private GUI gui;
    
    public OpenAction(GUI gui){
	super("Open");
	this.gui = gui;
    }
    
    public void actionPerformed(ActionEvent e) { 
        JFrame parent = gui.getJFrame();
	JFileChooser chooser = new JFileChooser();
	int returnVal = chooser.showOpenDialog(parent);
	if (returnVal == JFileChooser.APPROVE_OPTION) {
	    //String filename = chooser.getSelectedFile().getName();
	    gui.open(chooser.getSelectedFile());
	}
    }
}

