package net.pengo.app;

import java.awt.*;
import javax.swing.event.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.tree.*;

public class MoojMenuBar extends JMenuBar {
    protected final GUI gui;
    public MoojMenuBar(GUI gui) {
	super();
	this.gui = gui;
        setup();
    }
    protected void setup() {
        JMenu nowmenu; // temp menu holder
	nowmenu = new JMenu("File");
	nowmenu.add( new JMenuItem(new AbstractAction("Close") {
            public void actionPerformed(ActionEvent e) { 
                gui.closeAll();
            }
        }));
	nowmenu.add( new JMenuItem(new AbstractAction("Open") { 
            public void actionPerformed(ActionEvent e) { 
                gui.open();
            }
        }));
	nowmenu.add( new JMenuItem("Save")).setEnabled(false);
	nowmenu.add( new JMenuItem(new AbstractAction("Save as...") {
            public void actionPerformed(ActionEvent e) { 
                gui.saveAs();
            }
        }));
	nowmenu.add( new JMenuItem("Print...")).setEnabled(false);
	nowmenu.add( new JSeparator() );
	nowmenu.add( new JMenuItem(new ExitAction(gui))); // "Exit"
	this.add(nowmenu);

	nowmenu = new JMenu("Edit");
        nowmenu.add("no undo.").setEnabled(false);
	this.add(nowmenu);

	nowmenu = new JMenu("View");
	nowmenu.add( new JMenuItem(new AbstractAction("ASCII") {
            public void actionPerformed(ActionEvent e) { 
                gui.setGreyMode(0);
            }
        }));
	nowmenu.add( new JMenuItem(new AbstractAction("Grey scale") {
            public void actionPerformed(ActionEvent e) { 
                gui.setGreyMode(1);
            }
        }));
	nowmenu.add( new JMenuItem(new AbstractAction("Grey scale II") {
            public void actionPerformed(ActionEvent e) { 
                gui.setGreyMode(2);
            }
        }));
        this.add(nowmenu);

        nowmenu = new JMenu("Go");
        nowmenu.add("no go.").setEnabled(false);
	this.add(nowmenu);

	nowmenu = new JMenu("Tools");
        nowmenu.add("no tool.").setEnabled(false);
	this.add(nowmenu);

	nowmenu = new JMenu("Options");
        nowmenu.add("no option.").setEnabled(false);
	this.add(nowmenu);

	nowmenu = new JMenu("Help");
        nowmenu.add("no help.").setEnabled(false);
	this.add(nowmenu);
    }

}
