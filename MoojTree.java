import javax.swing.*;
import javax.swing.tree.*;
import javax.swing.event.*;
import java.util.*;
import java.awt.event.*;
import java.beans.*;

class MoojTree extends JTree implements ResourceListener, OpenFileListener {
    protected DefaultTreeModel treemodel;
    protected DefaultMutableTreeNode topnode; // the top node of the tree on the left, which the data goes under

    protected List openFileList = new ArrayList(); // open files to display (+ their resources)
    protected List topNodeList = new ArrayList(); // TreeNode wrapped versions of above. keep in sync.
    
    protected List resList = new ArrayList(); // resources
    protected List resNodeList = new ArrayList(); // resources as nodes

    protected final MoojTree thisTree = this; // needed for mouse adapter


    // constructor
    public static MoojTree create() {
        return create(null);
    }

    public static MoojTree create(OpenFile openFile) {
        DefaultMutableTreeNode topnode = new DefaultMutableTreeNode("mooj",true);
	DefaultTreeModel treemodel = new DefaultTreeModel(topnode);
	return new MoojTree(openFile, treemodel, topnode);
    }

    private MoojTree(OpenFile openFile, DefaultTreeModel treemodel, DefaultMutableTreeNode topnode) {
	super(treemodel);
        
	this.treemodel = treemodel;
	this.topnode = topnode;

	getSelectionModel().setSelectionMode(TreeSelectionModel.SINGLE_TREE_SELECTION); //XXX: for now

	//setRootVisible(false); //XXX: re-enable.. Why won't it work?
	setShowsRootHandles(true);
	setEditable(true);

        //testRenameTest(); // debugging

        if (openFile != null)
            addOpenFile(openFile);


	// click on tree:
	addTreeSelectionListener(new TreeSelectionListener() {
	    public void valueChanged(TreeSelectionEvent treeEvent) {
		DefaultMutableTreeNode node = (DefaultMutableTreeNode)getLastSelectedPathComponent();
		if (node == null)
		    return;

		Object innerSelected = node.getUserObject();
                if (innerSelected instanceof Resource){
                    Resource res = (Resource)innerSelected;
                    res.clickAction();
		}
	    }
	});

	// popup menu
        // XXX: take menu from SectionedNode.getPopupMenu()
	MouseListener ml = new MouseAdapter() {
		public void mousePressed(MouseEvent e) {
		    final int selRow = thisTree.getRowForLocation(e.getX(), e.getY());
		    final TreePath selPath = thisTree.getPathForLocation(e.getX(), e.getY());
                    if (selPath == null)
                        return;
                    
                    final Object[] o = selPath.getPath();
                    final DefaultMutableTreeNode selected = (DefaultMutableTreeNode)o[o.length-1];
                    final Object innerSelected = selected.getUserObject();

		    if(selRow != -1) {
                        if (e.getClickCount() == 2 && e.isMetaDown() == false) {
                            if (innerSelected instanceof Resource){
                                Resource res = (Resource)innerSelected;
                                res.doubleClickAction();
                            }
                        } else if (e.isPopupTrigger() || e.isMetaDown() ) {//XXX: popup trigger is never true? why?
                            setLeadSelectionPath(new TreePath(o)); // highlight (ant trail) selection. XXX: doesn't always work
                            setAnchorSelectionPath(new TreePath(o)); // highlight selection. XXX: doesn't always work
                            JPopupMenu popup;
                            /* // put this back if Resource becomes a type of DefaultMutableTreeNode (unlikely)
                            if (selected instanceof Resource) {
                                popup = ((Resource)selected).getJMenu().getPopupMenu();
                            } else */
                             if (innerSelected instanceof Resource) {
                                popup = ((Resource)innerSelected).getJMenu().getPopupMenu();
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

    }
    
    // find out what's happening when someone renames a tree element!
    public void testRenameTest() {
        if (getCellEditor() != null) {
            System.out.println( "ed: " + getCellEditor() + " .. " + getCellEditor().getCellEditorValue());
        }
        
        this.addPropertyChangeListener( new PropertyChangeListener(){
            public void propertyChange(PropertyChangeEvent evt) {
                System.out.println(evt.getPropertyName() + " -- " + evt.getOldValue() + " -> " + evt.getNewValue());
            }
        });
        
        this.addVetoableChangeListener( new VetoableChangeListener(){
            public void vetoableChange(PropertyChangeEvent evt)  {
                System.out.println("Vetoable! " + evt.getPropertyName() + " -- " + evt.getOldValue() + " -> " + evt.getNewValue());
            }
        });
    }

    // add a viewed file, effectively:
    public void addOpenFile(OpenFile openFile) {
	//XXX: error checking?

        openFile.addResourceListener(this);
        openFile.addOpenFileListener(this);
	openFileList.add(openFile);
        
        // wrap it!
        DefaultMutableTreeNode openFileNode = new DefaultMutableTreeNode(openFile,true);
        topNodeList.add(openFileNode);
        
	treemodel.insertNodeInto(openFileNode, topnode, topnode.getChildCount()); 
	//setRootVisible(false); // uncomment! grr.
    }
    
    public void fileClosed(FileEvent e) {
        removeOpenFile(e.getOpenFile());
    }


    public void removeOpenFile(OpenFile openFile) {
	if (openFile == null)
	    return;

        openFile.removeResourceListener(this);
        openFile.removeOpenFileListener(this);
	int index = openFileList.indexOf(openFile);
        openFileList.remove(index);
        DefaultMutableTreeNode treenode = (DefaultMutableTreeNode)topNodeList.remove(index);
        treemodel.removeNodeFromParent(treenode);
        
	if (openFileList.isEmpty()) {
	    setRootVisible(true);
	}
    }
    
    public void resourceAdded(ResourceEvent e) {
        //xxx: shouldn't really ignore category, but will for now
        Resource res = e.getResource();
        String cat = e.getCategory();
        OpenFile of = res.getOpenFile();
        
        DefaultMutableTreeNode resNode = new DefaultMutableTreeNode(res); //XXX: need res.canHaveChildren()?
        
        resList.add(res);
        resNodeList.add(resNode);
        
        int index = openFileList.indexOf(of);
        DefaultMutableTreeNode treenode = (DefaultMutableTreeNode)topNodeList.get(index);
        treemodel.insertNodeInto(resNode,treenode,treenode.getChildCount()); // XXX: position should be explicit        
    }
    
    public void resourceMoved(ResourceEvent e) {
    }
    
    public void resourceRemoved(ResourceEvent e) {
        Resource res = e.getResource();
        String cat = e.getCategory();
        OpenFile of = res.getOpenFile();
        int index = resList.indexOf(res);
        Object resNode = resNodeList.get( index );
        
        treemodel.removeNodeFromParent((MutableTreeNode)resNode);
        resList.remove(index);
        resNodeList.remove(index);
    }
    
    public void dataEdited(EditEvent e) {
    }
    
    public void dataLengthChanged(EditEvent e) {
    }
    
    public void definitionMade(DefinitionEvent e) {
    }
    
    public void definitionRemoved(DefinitionEvent e) {
    }
    
    public void fileSaved(FileEvent e) {
    }
    
    public void selectionCopied(ClipboardEvent e) {
    }
    
    public void selectionMade(SelectionEvent e) {
    }
    
    public void selectionRemoved(SelectionEvent e) {
    }
    
    public void resourcePropertyChanged(ResourceEvent e) {
    }
    
    public void resourceTypeChanged(ResourceEvent e) {
    }
    
}


class DeleteDefinitionAction extends AbstractAction {
    protected TreePath selPath;

    public DeleteDefinitionAction(TreePath selPath) {
	super("Delete definition");
	this.selPath = selPath;
    }

    public void actionPerformed(ActionEvent e) {
        /*
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
        */

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