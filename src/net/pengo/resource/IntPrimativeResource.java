/**
 * IntPrimativeResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.math.BigInteger;
import java.io.IOException;
import net.pengo.app.OpenFile;
import net.pengo.propertyEditor.IntPrimativeResourcePropertiesForm;
import net.pengo.dependency.QNode;

public class IntPrimativeResource extends IntResource
{
	

    private BigInteger value;
    
    public IntPrimativeResource(OpenFile openFile, BigInteger value) {
	super(openFile);
	this.value = value;
    }

    public IntPrimativeResource(OpenFile openFile, long value) {
	this(openFile, BigInteger.valueOf(value) );
    }

    public boolean isPrimative() {
	return true;
    }
    
	public QNode[] getSources()
	{
		// primatives can have no sources
		return new QNode[]{};
	}
	
    public BigInteger getValue() {
	return value;
    }
    
    public void setValue(BigInteger value) {
	value = this.value;
    }

    public void editProperties() {
	new IntPrimativeResourcePropertiesForm(this).show();
    }
    
}

