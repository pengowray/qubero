import java.util.*;
import javax.swing.*;
import javax.swing.tree.*;
import javax.swing.event.*;

/**
 * A node belonging to a SectionHeader
 */


 // obsoleted by "Resource"


interface SectionedNode extends MutableTreeNode {

    public JPopupMenu getPopupMenu();
    /*
	JMenu menu = new JMenu("Example");
	menu.add(new JMenuItem("sock puppets are fun!"));
	JPopupMenu popup = menu.getPopupMenu();
	return popup;
    */
    // perhaps the popup menu items can be introspectively generated. i.e. have a bean pattern for popup-able methods? do this much later.

    // how to respond to a rename event (XXX: these don't actually exist yet! argh)
    public void rename(String name);

    // drag and drop?

    // keyboard keys (delete/copy/etc)

    // move..?
}
