import java.util.*;
import javax.swing.*;
import javax.swing.tree.*;
import javax.swing.event.*;

/**
 * A node belonging to a the "Selection" DefSection
 */
class SectionedSelectionNode extends DefaultMutableTreeNode implements SectionedNode {
    
    public SectionedSelectionNode(RawDataSelection userObject) {
	super(userObject);
    }

    public JPopupMenu getPopupMenu() {
  	JMenu menu = new JMenu("Example");
	menu.add(new JMenuItem("define selection"));
	menu.add(new JMenuItem("and stuff!"));
	JPopupMenu popup = menu.getPopupMenu();
	return popup;
    }

    // how to respond to a rename event
    public void rename(String name) {
	//XXX
	return;
    }

}
