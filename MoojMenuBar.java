import java.awt.*;
import javax.swing.event.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.tree.*;

class MoojMenuBar extends JMenuBar {
    protected final HexEditorGUI gui;
    public MoojMenuBar(HexEditorGUI gui) {
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
	nowmenu.add( new JMenuItem(new AbstractAction("ASCII") {
            public void actionPerformed(ActionEvent e) { 
                gui.setGreyMode(false);
            }
        }));
        this.add(nowmenu);
	nowmenu.add( new JMenuItem(new AbstractAction("Grey scale") {
            public void actionPerformed(ActionEvent e) { 
                gui.setGreyMode(true);
            }
        }));

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
