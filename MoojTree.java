import javax.swing.*;
import javax.swing.tree.*;
import javax.swing.event.*;
import java.util.*;
import java.awt.event.*;


class MoojTree extends JTree {
    protected HexPanel hexpanel = null;

    protected DefaultTreeModel treemodel;
    protected DefaultMutableTreeNode topnode; // the top node of the tree on the left, which the data goes under

    protected List defNodeList = new ArrayList(); // nodes for the display of data definitions + their selection
    protected final MoojTree thisTree = this;

    // constructor
    public static MoojTree create(DefNode defnode) {
        DefaultMutableTreeNode topnode = new DefaultMutableTreeNode("mooj",true);

	DefaultTreeModel treemodel = new DefaultTreeModel(topnode);
	return new MoojTree(treemodel, defnode, topnode);
    }

    private MoojTree(DefaultTreeModel treemodel, DefNode defnode, DefaultMutableTreeNode topnode) {
	super(treemodel);

	this.treemodel = treemodel;
	this.topnode = topnode;

	getSelectionModel().setSelectionMode(TreeSelectionModel.SINGLE_TREE_SELECTION); //XXX: for now

	//setRootVisible(false); //XXX: re-enable.. Why won't it work?
	setShowsRootHandles(true);
	setEditable(true);

        addDefNode(defnode);


	// click on tree:
	addTreeSelectionListener(new TreeSelectionListener() {
	    public void valueChanged(TreeSelectionEvent treeEvent) {
		DefaultMutableTreeNode node = (DefaultMutableTreeNode)getLastSelectedPathComponent();

		if (node == null)
		    return;

		Object nodeInfo = node.getUserObject();

		if (nodeInfo instanceof RawDataSelection){
		    hexpanel.setSelection((RawDataSelection)nodeInfo);
		}

	    }
	});

	// popup menu
        // XXX: take menu from SectionedNode.getPopupMenu()
	MouseListener ml = new MouseAdapter() {
		public void mousePressed(MouseEvent e) {
		    final int selRow = thisTree.getRowForLocation(e.getX(), e.getY());
		    final TreePath selPath = thisTree.getPathForLocation(e.getX(), e.getY());
                    final Object[] o = selPath.getPath();
                    final Object selected = o[o.length-1];

		    if(selRow != -1) {
			if(e.isPopupTrigger() || e.isMetaDown() ) {//XXX: popup trigger is never true? why?
                            JPopupMenu popup;
                            if (selected instanceof SectionedNode) {
                                popup = ((SectionedNode)selected).getPopupMenu();
                            } else {
                                // default popup menu
                                // XXX: should there be a default?
                                JMenu pop = new JMenu("test");
                                Action aa = new AddToTemplateAction(selPath);
                                Action da = new DeleteDefinitionAction(selPath);
                                Action ia = new InfoAction(selPath);
                                //Action mua = new MoveUpAction(selPath);
                                //Action mda = new MoveDownAction(selPath);
                                pop.add(aa);
                                pop.add(da);
                                pop.add(ia);
                                //pop.add(mua);
                                //pop.add(mda);
                                pop.add(new JSeparator());
                                pop.add(new JMenuItem("sock puppets are fun!"));
                                popup = pop.getPopupMenu();
                            }
			    popup.show(thisTree, e.getX(), e.getY());
			}
		    }
		}
	    };
	addMouseListener(ml);
 
	// rename (edit) element:
	treemodel.addTreeModelListener(new TreeModelListener() {
	    public void treeNodesChanged(TreeModelEvent e) {
		//XXX: this is broken. changed node becomes a string!
		Object[] path = e.getPath();
		Object object = path[path.length-1];
		if (object instanceof RawDataSelection) {
		    //XXX: check that it's a selection (not a definition)
		    //if (path.length >= 2 && path[]);

		    RawDataSelection raw = (RawDataSelection)object;
		    DefNode defnode = raw.getDefNode();
		    
		    defnode.definitionMade(raw);
		    //defnode.clearSelection();
		}
	    }
	    public void treeNodesInserted(TreeModelEvent e){}
	    public void treeNodesRemoved(TreeModelEvent e){}
	    public void treeStructureChanged(TreeModelEvent e){}
	});

    }

    // add a viewed file, effectively:
    public void addDefNode(DefNode defNode) {
	//XXX: error checking
	defNode.setParentTreeModel(treemodel);
	defNodeList.add(defNode);
	treemodel.insertNodeInto(defNode, topnode, 0); // XXX: put at end, not begining?
	//setRootVisible(false); // uncomment! grr.
    }

    public void removeDefNode(DefNode defNode) {
	if (defNode == null) 
	    return;

	defNodeList.remove(defNode);
	treemodel.removeNodeFromParent(defNode);
	if (defNodeList.isEmpty()) {
	    setRootVisible(true);
	}
    }

    public void setHexPanel(HexPanel hexpanel) {
	this.hexpanel = hexpanel;
    }

}



class AddToTemplateAction extends AbstractAction {
    protected TreePath selPath;

    public AddToTemplateAction(TreePath selPath) {
	super("Add to template");
	this.selPath = selPath;
    }

    public void actionPerformed(ActionEvent e) {
	//XXX: this code is almost duplicated from above
	Object[] path = selPath.getPath();
	Object object = selPath.getLastPathComponent();
	Object userObject = null;

	if (object instanceof DefaultMutableTreeNode) {
	    userObject = ((DefaultMutableTreeNode)object).getUserObject();
	}

	if (userObject instanceof RawDataSelection) {
	    //XXX: different menus for selection and definition
	    //if (path.length >= 2 && path[]);
	    
	    RawDataSelection raw = (RawDataSelection)userObject;
	    DefNode defnode = raw.getDefNode();
	    
	    defnode.definitionMade(raw);
	    //defnode.clearSelection();
	}
	
    }
}




class DeleteDefinitionAction extends AbstractAction {
    protected TreePath selPath;

    public DeleteDefinitionAction(TreePath selPath) {
	super("Delete definition");
	this.selPath = selPath;
    }

    public void actionPerformed(ActionEvent e) {
	//XXX: this code is almost duplicated from above
	Object[] path = selPath.getPath();
	Object object = selPath.getLastPathComponent();
	Object userObject = null;

	if (object instanceof DefaultMutableTreeNode) {
	    userObject = ((DefaultMutableTreeNode)object).getUserObject();

	    if (userObject instanceof RawDataSelection) {
		//XXX: different menus for selection and definition
		//if (path.length >= 2 && path[]);
		
		RawDataSelection raw = (RawDataSelection)userObject;
		DefNode defnode = raw.getDefNode();
		
		defnode.deleteDefinition((DefaultMutableTreeNode)object);
		//defnode.clearSelection();
	    }
	}

    }
}

class InfoAction extends AbstractAction {
    protected TreePath selPath;

    public InfoAction(TreePath selPath) {
	super("Show info (Debug)");
	this.selPath = selPath;
    }

    public void actionPerformed(ActionEvent e) {
	Object object = selPath.getLastPathComponent();
	Object userObject = null;
	System.out.println("this is: " + object + " -- type: " + object.getClass());	

	if (object instanceof DefaultMutableTreeNode) {
	    userObject = ((DefaultMutableTreeNode)object).getUserObject();
	    System.out.println("user object: " + userObject + " -- type: " + userObject.getClass());
	}
    }
}

/*
abstract class NodeAction extends AbstractAction() {

    public PathAction (String name, Icon icon, TreePath selPath) {
	super(name, icon);
	this.selPath = selPath;
    }

    public PathAction (String name, TreePath selPath) {
	super(name);
	this.selPath = selPath;
    }
}


// do something with a tree node.
abstract class NodeVisitor {
    public NodeAction (TreePath selPath) {
	this.selPath = selPath;
    }

    public void actionPerformed(TreePath selPath) {
	Object[] path = selPath.getPath();
	Object object = selPath.getLastPathComponent();
	Object userObject = null;

	if (object instanceof DefaultMutableTreeNode) {
	    userObject = ((DefaultMutableTreeNode)object).getUserObject();

	    if (userObject instanceof RawDataSelection) {
		RawDataSelection raw = (RawDataSelection)userObject;
		DefNode defnode = raw.getDefNode();
		
		
		defnode.deleteDefinition((DefaultMutableTreeNode)object);
		actionPerformedOnDefinition(e, raw); //XXX: blah
	    }
	}
    }

    public void visitSelection(RawDataSelection raw){}

    public void visitDefinition(RawDataSelection raw){}

    public void visitEditedArea(ActionEvent e, RawDataSelection raw){}

}
*/