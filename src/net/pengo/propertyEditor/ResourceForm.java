/*
 * IntResourceForm.java
 *
 * Created on July 9, 2003, 10:21 PM
 *
 * another attempt at IntInputBox
 */

package net.pengo.propertyEditor;

 
import javax.swing.JTree;
import javax.swing.tree.DefaultMutableTreeNode;

import net.pengo.resource.Resource;

/**
 *
 * @author  Smiley
 */
public class ResourceForm extends PropertiesForm {
    //private OpenFile openFile;
    private Resource res;
    
    public ResourceForm(Resource res) {
        super();
        this.res = res;
    }
    
    protected synchronized JTree getMenu() {
        
        DefaultMutableTreeNode root = deepSources(res);
        JTree jt = new JTree(root);
        jt.setSelectionPath(jt.getPathForRow(0));
        return jt;
    }
    
    public DefaultMutableTreeNode deepSources(Resource r) {
        
        //fixme: put all primaries onto one main page and make that the root
    	PropertyPage[] pp = (PropertyPage[]) r.getPrimaryPages().toArray(new PropertyPage[0]);
    	MultiPage mp = new MultiPage(this, pp, r.nameOrType());
    	DefaultMutableTreeNode resNode = new DefaultMutableTreeNode(mp);
    	
    	// add pages as seperate nodes 
    	// ...(dont bother.. we'll do them as a multipage)
//    	for (Iterator iter = pp.iterator(); iter.hasNext();) {
//    	    PropertyPage page = (PropertyPage) iter.next();
//    	    page.setForm(this);
//    	    DefaultMutableTreeNode dmtn = new DefaultMutableTreeNode(page);
//    	    resNode.add(dmtn);
//    	    
//    	}
    	
    	Resource[] source = r.getSources();
    	for (int i = 0; i < source.length; i++) {
	        Resource resource = source[i];
	        //System.out.println("diving into:" + resource);
	        resNode.add(deepSources(resource));
	    }
	    
	    return resNode; 
    }
	
    
    
    
}
