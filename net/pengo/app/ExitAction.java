package net.pengo.app;

import javax.swing.AbstractAction;
import java.awt.event.ActionEvent;

public class ExitAction extends AbstractAction {
    GUI gui;
    public ExitAction(GUI gui){
	super("Exit");
	this.gui = gui;
    }
    public void actionPerformed(ActionEvent e) { 
	gui.quit();
    }
}

