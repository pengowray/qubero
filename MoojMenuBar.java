import java.awt.*;
import javax.swing.event.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.tree.*;

class MoojMenuBar extends JMenuBar {
    protected HexEditorGUI gui;
    public MoojMenuBar(HexEditorGUI gui) {
	super();
	this.gui = gui;
	JMenu nowmenu; // temp menu holder

	nowmenu = new JMenu("File");
	nowmenu.add( new JMenuItem("New"));
	nowmenu.add( new JMenuItem(new OpenAction(gui))); // "Open"
	nowmenu.add( new JMenuItem("Save"));
	nowmenu.add( new JMenuItem("Save as..."));
	nowmenu.add( new JMenuItem("Print..."));
	nowmenu.add( new JSeparator() );
	nowmenu.add( new JMenuItem(new ExitAction(gui))); // "Exit"
	this.add(nowmenu);

	nowmenu = new JMenu("Edit");
	this.add(nowmenu);

	nowmenu = new JMenu("View");
	this.add(nowmenu);

	nowmenu = new JMenu("Go");
	this.add(nowmenu);

	nowmenu = new JMenu("Tools");
	this.add(nowmenu);

	nowmenu = new JMenu("Options");
	this.add(nowmenu);

	nowmenu = new JMenu("Help");
	this.add(nowmenu);
    }

}
