
 /**
 * listEditor.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.listEditor;

import net.pengo.propertyEditor.PropertiesForm;
import net.pengo.resource.ListResource;
import net.pengo.resource.Resource;

import javax.swing.JTree;
import javax.swing.tree.DefaultMutableTreeNode;
import java.util.Iterator;
import net.pengo.propertyEditor.ResourceMultiPage;

public class ListEditor extends PropertiesForm
{
	private ListResource list;
	
	public ListEditor(ListResource listResource) {
		super();
		
		this.list = listResource;
	}
	
	protected JTree getMenu()
	{
		
        DefaultMutableTreeNode root = new DefaultMutableTreeNode(new ListPage(this, list));
		
		for (Iterator i = list.iterator(); i.hasNext(); ) {
			Resource r = (Resource)i.next();
			root.add(deepSources(r));
		}
			
        JTree jt = new JTree(root);
        jt.setSelectionPath(jt.getPathForRow(0));
        return jt;
	}

	//* warning/fixme: taken directly from ResourceForm */
    public DefaultMutableTreeNode deepSources(Resource r) {
        
        ResourceMultiPage mp = new ResourceMultiPage(this, r);
    	DefaultMutableTreeNode resNode = new DefaultMutableTreeNode(mp);
    	
    	Resource[] source = r.getSources();
    	for (int i = 0; i < source.length; i++) {
	        Resource resource = source[i];
	        //System.out.println("diving into:" + resource);
	        resNode.add(deepSources(resource));
	    }
	    
	    return resNode;
    }

}

