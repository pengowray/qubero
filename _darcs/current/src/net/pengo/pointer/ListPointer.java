/**
 * ListPointer.java
 *
 * A pointer to an item in a ListResource
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.pointer;

import net.pengo.resource.Resource;
import net.pengo.resource.IntResource;
import net.pengo.resource.ListResource;


public class ListPointer extends Resource
{
	//fixme: should this subtype SmartPointer ?
	
    public final SmartPointer list = new JavaPointer("net.pengo.resource.ListResource");
    public final SmartPointer index = new JavaPointer("net.pengo.resource.IntResource");
	
	public ListPointer(ListResource list, IntResource index) {
        super();
        
        list.addSink(this);
        list.setName("List");
        setList(list);
        
        index.addSink(this);
        index.setName("Index");
        setIndex(index);
	}
	
	public Resource[] getSources()
	{
		return new Resource[] { list, index };
	}
	
	public Resource evaluate() {
		return getList().elementAt(getIndex());
	}
	
	public ListResource getList() {
		return ((ListResource)list.evaluate());
	}
	
	public void setList(ListResource list) {
		this.list.setValue(list);
	}
	
	public IntResource getIndex() {
		return (IntResource)index.evaluate();
	}

	public void setIndex(IntResource index) {
		this.index.setValue(index);
	}
	
    public boolean isReference() {
		//fixme: i guess it's a reference?
        return true;
    }
	
    public boolean isPrimative() {
        return false;
    }
}

