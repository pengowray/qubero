/**
 * DefaultDepLink.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.dependency;

import net.pengo.resource.*;

public class DefaultDepLink
{
    private Resource source;
    private Resource sink;
    
    /**
     * Constructor
     *
     * @param    source              a  QNode
     * @param    sink                a  QNode
     */
    public DefaultDepLink(Resource source, Resource sink) {
	this.source = source;
	this.sink = sink;
    }
    
    /**
     * Sets Source
     *
     * @param    Source              a  QNode
     */
    public void setSource(Resource source) {
	this.source = source;
    }
    
    /**
     * Returns Source
     *
     * @return    a  QNode
     */
    public Resource getSource() {
	return source;
    }
    
    /**
     * Sets Sink
     *
     * @param    Sink                a  QNode
     */
    public void setSink(Resource sink) {
	this.sink = sink;
    }
    
    /**
     * Returns Sink
     *
     * @return    a  QNode
     */
    public Resource getSink() {
	return sink;
    }
}
    

