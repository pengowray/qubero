package net.pengo.app;

import javax.swing.AbstractAction;
import java.awt.event.ActionEvent;

public class OpenAction extends AbstractAction {
    GUI gui;
    public OpenAction(GUI gui){
	super("Open");
	this.gui = gui;
    }
    public void actionPerformed(ActionEvent e) { 
	gui.open();
    }
}

